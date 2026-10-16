use super::*;
use crate::test_support::{git_success, init_test_repo, open_repo};

#[test]
fn chunked_reads_retry_interruptions_and_remain_cancellable() {
    struct InterruptedReader {
        interruptions: usize,
        cancellation: Option<CancellationToken>,
        bytes: &'static [u8],
    }
    impl Read for InterruptedReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.interruptions > 0 {
                self.interruptions -= 1;
                if let Some(token) = &self.cancellation {
                    token.cancel();
                }
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            self.bytes.read(buffer)
        }
    }
    let mut reader = InterruptedReader {
        interruptions: 3,
        cancellation: None,
        bytes: b"complete contents",
    };
    let mut output = Vec::new();
    copy_and_hash(
        &mut reader,
        &mut output,
        &mut FxHasher::default(),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(output, b"complete contents");

    let token = CancellationToken::new();
    let mut reader = InterruptedReader {
        interruptions: 3,
        cancellation: Some(token.clone()),
        bytes: b"never read",
    };
    let error = read_chunk_cancellable(&mut reader, &mut [0; 32], &token).unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::Cancelled));
    assert_eq!(
        reader.interruptions, 2,
        "cancellation is checked between retries"
    );
}

#[cfg(windows)]
#[test]
fn windows_memo_checks_allow_in_place_saves_and_replacements() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("identity.txt");
    std::fs::write(&path, b"before").unwrap();
    let _clock = super::super::RacyClockSkew::set(std::time::Duration::from_secs(30));
    // Kept alive across the edits: a lookup holding a handle would block them.
    let _stamp = DiskFileStamp::read_for_verification_memo(&path);
    let mut writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("memo lookup must allow in-place editor saves");
    writer.write_all(b"edited").unwrap();
    drop(writer);
    let replacement = tmp.path().join("replacement.txt");
    std::fs::write(&replacement, b"replacement").unwrap();
    std::fs::rename(&replacement, &path).expect("memo lookup must allow atomic saves");
    std::fs::remove_file(&path).expect("memo lookup must allow deletion");
}

#[cfg(windows)]
#[test]
fn windows_preview_verification_rechecks_same_length_edit_with_open_writer() {
    let tmp = tempfile::tempdir().unwrap();
    init_test_repo(tmp.path());
    let blob_id = stage_blob(tmp.path(), "asset.bin", b"real blob bytes");
    let repo = open_repo(tmp.path());
    let cache = repo
        .cached_preview_blob_file_path(blob_id, Path::new("asset.bin"))
        .unwrap()
        .unwrap();
    let _clock = super::super::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let handle = repo.repo();
    let token = CancellationToken::new();
    assert!(repo.cached_preview_blob_matches(&handle, &cache, blob_id, &token));
    assert!(
        repo.preview_blob_verified.lock().unwrap().is_empty(),
        "Windows must verify preview contents even outside the timestamp race window"
    );
    let modified = std::fs::metadata(&cache).unwrap().modified().unwrap();
    let mut writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&cache)
        .unwrap();
    assert!(repo.cached_preview_blob_matches(&handle, &cache, blob_id, &token));
    writer.write_all(b"fake blob bytes").unwrap();
    writer.set_modified(modified).unwrap();
    // Keep the writer open: Windows can defer metadata updates until close.
    assert!(!repo.cached_preview_blob_matches(&handle, &cache, blob_id, &token));
}

#[cfg(unix)]
#[test]
fn blob_copy_rejects_partial_output_with_exit_code_one() {
    let mut command = Command::new("sh");
    command.args(["-c", "printf partial; exit 1"]);
    let error = copy_git_stdout_to_temp_file(command, "git cat-file", &CancellationToken::new())
        .expect_err("a failed cat-file must not produce a cache candidate");
    assert!(
        !matches!(error.kind(), ErrorKind::Cancelled),
        "unexpected error kind: {error:?}"
    );
}

#[test]
fn cancelled_copy_stops_between_chunks_without_publishing_partial_content() {
    struct CancelReader(CancellationToken, usize);
    impl Read for CancelReader {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.1 += 1;
            bytes.fill(b'x');
            self.0.cancel();
            Ok(bytes.len())
        }
    }
    let token = CancellationToken::new();
    let mut reader = CancelReader(token.clone(), 0);
    let mut output = Vec::new();
    let error =
        copy_and_hash(&mut reader, &mut output, &mut FxHasher::default(), &token).unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::Cancelled));
    assert_eq!(reader.1, 1);
    assert!(output.is_empty());
}
use gitcomet_core::domain::{CommitId, DiffArea, DiffTarget};
use gitcomet_core::error::ErrorKind;

#[test]
fn attributes_fingerprint_keeps_lfs_and_drops_other_filter_drivers() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init_test_repo(root);
    std::fs::write(
        root.join(".gitattributes"),
        "*.bin filter=lfs\n*.raw filter=other\n*.txt text\n",
    )
    .unwrap();
    git_success(root, &["config", "filter.lfs.clean", "one"]);
    let repo = open_repo(root).repo();
    let lfs = worktree_attributes_fingerprint(&repo, Path::new("a.bin"));
    assert!(lfs.is_some(), "lfs output is a pure function of the bytes");
    assert!(worktree_attributes_fingerprint(&repo, Path::new("a.raw")).is_none());
    assert!(worktree_attributes_fingerprint(&repo, Path::new("a.txt")).is_some());
    assert_ne!(
        lfs,
        worktree_attributes_fingerprint(&repo, Path::new("a.txt"))
    );

    git_success(root, &["config", "filter.lfs.clean", "two"]);
    let repo = open_repo(root).repo();
    assert_ne!(
        worktree_attributes_fingerprint(&repo, Path::new("a.bin")),
        lfs,
        "a changed driver command must invalidate the memo"
    );
}

#[test]
fn worktree_diff_does_not_write_index() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init_test_repo(root);
    let file = root.join("file.txt");
    std::fs::write(&file, "unchanged content\n").unwrap();
    git_success(root, &["add", "file.txt"]);
    git_success(root, &["commit", "-m", "Initial"]);
    git_success(root, &["config", "diff.autoRefreshIndex", "true"]);
    let index = root.join(".git/index");
    let before = std::fs::read(&index).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(120)),
        )
        .unwrap();
    let output = open_repo(root)
        .build_unified_diff_command(&DiffTarget::working_tree(
            "file.txt".into(),
            DiffArea::Unstaged,
        ))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        std::fs::read(index).unwrap(),
        before,
        "read-only diff refreshed index stat metadata"
    );
}

/// A copied file's old side is its source, which `--find-renames` never
/// pairs: the patch printed the whole file as added while the text view
/// showed the edit against the source.
#[test]
fn commit_diff_pairs_a_copied_file_with_its_source() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    init_test_repo(root);
    let body: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    std::fs::write(root.join("a.rs"), &body).unwrap();
    git_success(root, &["add", "a.rs"]);
    git_success(root, &["commit", "-m", "seed"]);
    std::fs::write(
        root.join("b.rs"),
        body.replace("line 20\n", "line twenty\n"),
    )
    .unwrap();
    git_success(root, &["add", "b.rs"]);
    git_success(root, &["commit", "-m", "copy"]);

    let repo = open_repo(root);
    let head = super::super::history::gix_head_id_or_none(&repo.repo())
        .unwrap()
        .unwrap();
    let target = DiffTarget::commit(CommitId(head.to_string().into()), "b.rs".into())
        .with_old_path(Some("a.rs".into()));
    let patch = repo.diff_unified_impl(&target).unwrap();
    assert!(patch.contains("copy from a.rs\n"), "{patch}");
    assert!(patch.contains("-line 20\n+line twenty\n"), "{patch}");
    assert!(!patch.contains("new file mode"), "{patch}");
}

#[test]
fn read_worktree_image_file_bytes_rejects_oversized_file_before_reading() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("large.png");
    let file = std::fs::File::create(&path).expect("create sparse image");
    file.set_len(MAX_IMAGE_DIFF_SIDE_BYTES + 1)
        .expect("make sparse image oversized");

    let err = read_worktree_image_file_bytes_optional(tmp.path(), Path::new("large.png"))
        .expect_err("oversized image should fail");

    let ErrorKind::Backend(message) = err.kind() else {
        panic!("expected backend size-limit error, got {err:?}");
    };
    assert!(message.contains("image diff side"));
    assert!(message.contains("byte limit"));
}

#[test]
fn read_worktree_image_file_bytes_allows_file_at_size_limit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bytes = vec![7; MAX_IMAGE_DIFF_SIDE_BYTES as usize];
    std::fs::write(tmp.path().join("limit.png"), &bytes).expect("write image at limit");

    let loaded = read_worktree_image_file_bytes_optional(tmp.path(), Path::new("limit.png"))
        .expect("load image at limit")
        .expect("image at limit");

    assert_eq!(loaded, bytes);
}

#[test]
fn worktree_source_identity_changes_with_normalized_content_hash() {
    let workdir = Path::new("/tmp/repo");
    let path = Path::new("src/lib.rs");

    let first = worktree_source_identity(workdir, path, 0x11);
    let second = worktree_source_identity(workdir, path, 0x22);

    assert_ne!(first, second);
}

#[cfg(windows)]
#[test]
fn windows_worktree_verification_repairs_cache_and_observes_open_writer() {
    let tmp = tempfile::tempdir().unwrap();
    init_test_repo(tmp.path());
    let path = Path::new("memo.txt");
    stage_blob(tmp.path(), "memo.txt", b"correct content\n");
    let repo = open_repo(tmp.path());
    let _clock = super::super::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let source = repo
        .cached_git_normalized_worktree_file_source(&repo.repo(), path)
        .unwrap()
        .unwrap();
    repo.cached_git_normalized_worktree_file_source(&repo.repo(), path)
        .unwrap();
    let modified = std::fs::metadata(&source.path).unwrap().modified().unwrap();
    std::fs::write(&source.path, b"altered content\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&source.path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let reloaded = repo
        .cached_git_normalized_worktree_file_source(&repo.repo(), path)
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read(reloaded.path).unwrap(), b"correct content\n");
    let worktree_path = tmp.path().join(path);
    let modified = std::fs::metadata(&worktree_path)
        .unwrap()
        .modified()
        .unwrap();
    let mut writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&worktree_path)
        .unwrap();
    writer.write_all(b"changed content\n").unwrap();
    writer.set_modified(modified).unwrap();
    let changed = repo
        .cached_git_normalized_worktree_file_source(&repo.repo(), path)
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read(changed.path).unwrap(), b"changed content\n");
}

fn stage_blob(workdir: &Path, relative: &str, content: &[u8]) -> gix::ObjectId {
    std::fs::write(workdir.join(relative), content).expect("write file");
    git_success(workdir, &["add", relative]);
    gix::objs::compute_hash(
        gix::open(workdir).unwrap().object_hash(),
        gix::objs::Kind::Blob,
        content,
    )
    .expect("blob id")
}

#[test]
fn preview_blob_cache_ignores_pre_planted_file_with_other_content() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    let logical_path = Path::new("image.bin");
    let blob_id = stage_blob(tmp.path(), "image.bin", b"real blob bytes");
    let repo = open_repo(tmp.path());

    // The name is a function of (workdir, path, blob id) that anyone on the
    // host can compute, so a file there is not evidence of who wrote it.
    let cache_path = preview_blob_cache_path(&repo.spec.workdir, logical_path, &blob_id);
    std::fs::write(&cache_path, b"planted by someone else").expect("plant cache file");

    let served = repo
        .cached_preview_blob_file_path(blob_id, logical_path)
        .expect("materialize blob")
        .expect("blob exists");

    assert_eq!(served, cache_path);
    assert_eq!(
        std::fs::read(&served).expect("read served file"),
        b"real blob bytes"
    );
}

// Injecting a real memo stamp requires Unix; Windows always rehashes.
#[cfg(unix)]
#[test]
fn preview_blob_verification_memo_rechecks_matching_racy_stamp() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    let logical_path = Path::new("image.bin");
    let blob_id = stage_blob(tmp.path(), "image.bin", b"real blob bytes");
    let repo = open_repo(tmp.path());
    let cache_path = preview_blob_cache_path(&repo.spec.workdir, logical_path, &blob_id);
    std::fs::write(&cache_path, b"fake blob bytes").expect("tamper");
    // Keep the stamp racy regardless of how long the test is descheduled.
    std::fs::File::options()
        .write(true)
        .open(&cache_path)
        .expect("open cache")
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
        .expect("set future mtime");

    // Model a same-tick rewrite: the metadata still matches the memo, but
    // the content no longer matches the previously verified blob. Inject
    // the matching stamp so this also reproduces on fine-grained filesystems.
    repo.preview_blob_verified.lock().expect("memo").insert(
        cache_path.clone(),
        VerifiedPreviewBlob {
            file: DiskFileStamp::read(&cache_path).expect("stamp"),
            blob_id,
        },
    );

    let served = repo
        .cached_preview_blob_file_path(blob_id, logical_path)
        .expect("re-verify")
        .expect("blob exists");
    assert_eq!(
        std::fs::read(served).expect("read served"),
        b"real blob bytes"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn preview_blob_verification_memo_does_not_record_racy_hash_verification() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    let logical_path = Path::new("image.bin");
    let blob_id = stage_blob(tmp.path(), "image.bin", b"real blob bytes");
    let repo = open_repo(tmp.path());
    let cache_path = preview_blob_cache_path(&repo.spec.workdir, logical_path, &blob_id);
    std::fs::write(&cache_path, b"real blob bytes").expect("write cache");
    std::fs::File::options()
        .write(true)
        .open(&cache_path)
        .expect("open cache")
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
        .expect("set future mtime");

    let served = repo
        .cached_preview_blob_file_path(blob_id, logical_path)
        .expect("verify")
        .expect("blob exists");
    assert_eq!(served, cache_path);
    assert!(
        repo.preview_blob_verified.lock().expect("memo").is_empty(),
        "verification inside the race window must not become trusted as time passes"
    );
}

/// The stamp taken for the memo lookup is the one hashing must preserve, so
/// a verified miss stats the cache file once before hashing and once after.
#[test]
fn preview_blob_verification_stats_cache_file_once_before_hashing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    let blob_id = stage_blob(tmp.path(), "image.bin", b"real blob bytes");
    let repo = open_repo(tmp.path());
    let cache = repo
        .cached_preview_blob_file_path(blob_id, Path::new("image.bin"))
        .expect("materialize blob")
        .expect("blob exists");
    repo.preview_blob_verified.lock().expect("memo").clear();
    let _clock = super::super::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let handle = repo.repo();
    let token = CancellationToken::new();

    let stats = super::super::disk_file_stats_for_test();
    assert!(repo.cached_preview_blob_matches(&handle, &cache, blob_id, &token));
    assert_eq!(
        super::super::disk_file_stats_for_test() - stats,
        2,
        "a verified miss stats before and after hashing"
    );
    // Unix records the verified stamp; the next check is a memo hit.
    #[cfg(unix)]
    {
        let stats = super::super::disk_file_stats_for_test();
        assert!(repo.cached_preview_blob_matches(&handle, &cache, blob_id, &token));
        assert_eq!(
            super::super::disk_file_stats_for_test() - stats,
            1,
            "a memo hit stats once"
        );
    }
}

#[cfg(unix)]
#[test]
fn preview_blob_cache_replaces_symlink_at_cache_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    let logical_path = Path::new("image.bin");
    let blob_id = stage_blob(tmp.path(), "image.bin", b"real blob bytes");
    let repo = open_repo(tmp.path());

    let elsewhere = tempfile::tempdir().expect("symlink target dir");
    let target = elsewhere.path().join("target");
    std::fs::write(&target, b"real blob bytes").expect("write symlink target");
    let cache_path = preview_blob_cache_path(&repo.spec.workdir, logical_path, &blob_id);
    let _ = std::fs::remove_file(&cache_path);
    std::os::unix::fs::symlink(&target, &cache_path).expect("plant symlink");

    let served = repo
        .cached_preview_blob_file_path(blob_id, logical_path)
        .expect("materialize blob")
        .expect("blob exists");

    let metadata = std::fs::symlink_metadata(&served).expect("served metadata");
    assert!(
        metadata.file_type().is_file(),
        "a symlink at the cache path must be replaced by a regular file, even when its target matches"
    );
    assert_eq!(
        std::fs::read(&served).expect("read served file"),
        b"real blob bytes"
    );
}

#[test]
fn preview_blob_cache_keeps_verified_existing_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    let logical_path = Path::new("image.bin");
    let blob_id = stage_blob(tmp.path(), "image.bin", b"real blob bytes");
    let repo = open_repo(tmp.path());

    let first = repo
        .cached_preview_blob_file_path(blob_id, logical_path)
        .expect("materialize blob")
        .expect("blob exists");
    let mut permissions = std::fs::metadata(&first)
        .expect("first cache metadata")
        .permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&first, permissions).expect("make cache read-only");

    let second = repo
        .cached_preview_blob_file_path(blob_id, logical_path)
        .expect("reuse cached blob")
        .expect("blob exists");

    assert_eq!(first, second);
    let metadata = std::fs::metadata(&second).expect("second cache metadata");
    assert!(
        metadata.permissions().readonly(),
        "a verified cache file must be reused, not rewritten"
    );

    #[cfg(windows)]
    {
        let mut permissions = metadata.permissions();
        // Windows-only cleanup; this never changes Unix permission bits.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&second, permissions).expect("restore writable cache for cleanup");
    }
}

#[cfg(unix)]
#[test]
fn persist_worktree_git_cache_file_replaces_symlink_even_with_identical_content() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_path = tmp.path().join("gitcomet-diff-worktree-symlink.txt");
    let target = tmp.path().join("target.txt");
    let content = b"identical normalized content";
    std::fs::write(&target, content).expect("write symlink target");
    std::os::unix::fs::symlink(&target, &cache_path).expect("plant symlink");

    let mut duplicate = tempfile::NamedTempFile::new_in(tmp.path()).expect("temp file");
    duplicate.write_all(content).expect("write cache candidate");
    duplicate.flush().expect("flush cache candidate");

    persist_worktree_git_cache_file(duplicate, &cache_path).expect("replace symlinked cache file");

    let metadata = std::fs::symlink_metadata(&cache_path).expect("cache metadata");
    assert!(metadata.file_type().is_file());
    assert_eq!(std::fs::read(&cache_path).expect("read cache"), content);
    assert_eq!(
        std::fs::read(&target).expect("read former target"),
        content,
        "the symlink target must be left alone"
    );
}

#[test]
fn persist_worktree_git_cache_file_replaces_existing_cache_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_path = tmp.path().join("gitcomet-diff-worktree-test.txt");
    std::fs::write(&cache_path, b"old normalized content").expect("write stale cache");

    let mut replacement = tempfile::NamedTempFile::new_in(tmp.path()).expect("temp file");
    replacement
        .write_all(b"new normalized content")
        .expect("write replacement cache");
    replacement.flush().expect("flush replacement cache");

    persist_worktree_git_cache_file(replacement, &cache_path)
        .expect("replace stale normalized cache file");

    assert_eq!(
        std::fs::read(&cache_path).expect("read replaced cache"),
        b"new normalized content"
    );
}

#[test]
fn persist_worktree_git_cache_file_preserves_identical_existing_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_path = tmp.path().join("gitcomet-diff-worktree-identical.txt");
    let content = b"unchanged normalized content";
    std::fs::write(&cache_path, content).expect("write existing cache");
    let mut permissions = std::fs::metadata(&cache_path)
        .expect("existing cache metadata")
        .permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&cache_path, permissions).expect("make existing cache read-only");
    let identity_before = FileDiffTextSource::new(cache_path.clone()).identity;

    let mut duplicate = tempfile::NamedTempFile::new_in(tmp.path()).expect("temp file");
    duplicate
        .write_all(content)
        .expect("write identical cache candidate");
    duplicate.flush().expect("flush identical cache candidate");

    persist_worktree_git_cache_file(duplicate, &cache_path)
        .expect("keep identical normalized cache file");

    let metadata = std::fs::metadata(&cache_path).expect("preserved cache metadata");
    assert!(
        metadata.permissions().readonly(),
        "an identical refresh must retain the existing cache file, not replace it"
    );
    assert_eq!(
        FileDiffTextSource::new(cache_path.clone()).identity,
        identity_before,
        "an identical refresh must preserve the source freshness identity"
    );
    assert_eq!(std::fs::read(&cache_path).expect("read cache"), content);

    #[cfg(windows)]
    {
        let mut permissions = metadata.permissions();
        // Windows-only cleanup; this never changes Unix permission bits.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&cache_path, permissions)
            .expect("restore writable cache for cleanup");
    }
}

#[test]
fn repeated_worktree_source_load_preserves_content_addressed_cache_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    let logical_path = Path::new("src/lib.rs");
    std::fs::create_dir_all(tmp.path().join("src")).expect("create source directory");
    std::fs::write(tmp.path().join(logical_path), b"fn unchanged() {}\n")
        .expect("write worktree source");
    let repo = open_repo(tmp.path());
    let thread_local_repo = repo.repo();

    let first = repo
        .cached_git_normalized_worktree_file_source(&thread_local_repo, logical_path)
        .expect("load first normalized source")
        .expect("worktree source");
    let mut permissions = std::fs::metadata(&first.path)
        .expect("first cache metadata")
        .permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&first.path, permissions).expect("make cache read-only");
    let freshness_before = FileDiffTextSource::new(first.path.clone()).identity;

    let second = repo
        .cached_git_normalized_worktree_file_source(&thread_local_repo, logical_path)
        .expect("reload identical normalized source")
        .expect("reloaded worktree source");

    assert_eq!(
        second, first,
        "normalized content identity should be stable"
    );
    let metadata = std::fs::metadata(&second.path).expect("reloaded cache metadata");
    assert!(
        metadata.permissions().readonly(),
        "a no-op source reload must retain the existing cache file"
    );
    assert_eq!(
        FileDiffTextSource::new(second.path.clone()).identity,
        freshness_before,
        "a no-op source reload must preserve the UI freshness identity"
    );

    #[cfg(windows)]
    {
        let mut permissions = metadata.permissions();
        // Windows-only cleanup; this never changes Unix permission bits.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&second.path, permissions)
            .expect("restore writable cache for cleanup");
    }
    std::fs::remove_file(&second.path).expect("remove content-addressed test cache");
}

#[test]
fn diff_file_text_for_staged_gitlink_returns_empty_sources() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    git_success(
        tmp.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!(
                "160000,{},vendor/sub",
                "1".repeat(gix::open(tmp.path()).unwrap().object_hash().len_in_hex())
            ),
        ],
    );

    let repo = open_repo(tmp.path());
    let diff = repo
        .diff_file_text_impl(&DiffTarget::working_tree(
            "vendor/sub".into(),
            DiffArea::Staged,
        ))
        .expect("gitlink text diff should not error")
        .expect("file diff text object");

    assert_eq!(diff.path, Path::new("vendor/sub"));
    assert!(diff.old_source.is_none());
    assert!(diff.new_source.is_none());
}
