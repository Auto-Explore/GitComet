use super::*;

pub(crate) fn empty_history_graph_heads<'a>() -> HashSet<&'a str> {
    HashSet::default()
}

pub(crate) fn history_graph_heads_from_indices<'a>(
    commits: &'a [Commit],
    branch_head_indices: &[usize],
) -> HashSet<&'a str> {
    branch_head_indices
        .iter()
        .filter_map(|&ix| commits.get(ix).map(|commit| commit.id.as_ref()))
        .collect()
}

pub(crate) fn history_graph_heads_from_branches<'a>(
    branches: &'a [Branch],
    remote_branches: &'a [RemoteBranch],
) -> HashSet<&'a str> {
    branches
        .iter()
        .map(|branch| branch.target.as_ref())
        .chain(remote_branches.iter().map(|branch| branch.target.as_ref()))
        .collect()
}

pub(in crate::view) fn prepare_bench_diff_syntax_document(
    language: DiffSyntaxLanguage,
    budget: DiffSyntaxBudget,
    text: &str,
    old_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
    let text: SharedString = text.to_owned().into();
    let line_starts: Arc<[usize]> = Arc::from(line_starts_for_text(text.as_ref()));

    prepare_bench_diff_syntax_document_from_shared(
        language,
        budget,
        text,
        line_starts,
        old_document,
    )
}

pub(in crate::view) fn prepare_bench_diff_syntax_document_from_shared(
    language: DiffSyntaxLanguage,
    budget: DiffSyntaxBudget,
    text: SharedString,
    line_starts: Arc<[usize]>,
    old_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
    match prepare_diff_syntax_document_with_budget_reuse_text(
        language,
        DiffSyntaxMode::Auto,
        text.clone(),
        Arc::clone(&line_starts),
        budget,
        old_document,
        None,
    ) {
        PrepareDiffSyntaxDocumentResult::Ready(document) => Some(document),
        PrepareDiffSyntaxDocumentResult::TimedOut => {
            let old_reparse_seed = old_document.and_then(prepared_diff_syntax_reparse_seed);
            prepare_diff_syntax_document_in_background_text_with_reuse(
                language,
                DiffSyntaxMode::Auto,
                text,
                line_starts,
                old_reparse_seed,
                None,
            )
            .map(inject_background_prepared_diff_syntax_document)
        }
        PrepareDiffSyntaxDocumentResult::Unsupported => None,
    }
}

pub(crate) fn build_synthetic_repo_state(
    local_branches: usize,
    remote_branches: usize,
    remotes: usize,
    worktrees: usize,
    submodules: usize,
    stashes: usize,
    commits: &[Commit],
) -> RepoState {
    let id = RepoId(1);
    let spec = RepoSpec {
        workdir: std::path::PathBuf::from("/tmp/bench"),
    };
    let mut repo = RepoState::new_opening(id, spec);

    let head = "main".to_string();
    repo.head_branch = Loadable::Ready(head.clone());

    let target = commits
        .first()
        .map(|c| c.id.clone())
        .unwrap_or_else(|| CommitId("0".repeat(40).into()));

    let mut branches = Vec::with_capacity(local_branches.max(1));
    branches.push(Branch {
        name: head.clone(),
        target: target.clone(),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: head.clone(),
        }),
        divergence: Some(UpstreamDivergence {
            ahead: 1,
            behind: 2,
        }),
    });
    for ix in 0..local_branches.saturating_sub(1) {
        branches.push(Branch {
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target: target.clone(),
            upstream: None,
            divergence: None,
        });
    }
    repo.branches = Loadable::Ready(Arc::new(branches));

    let mut remotes_vec = Vec::with_capacity(remotes.max(1));
    for r in 0..remotes.max(1) {
        remotes_vec.push(Remote {
            name: if r == 0 {
                "origin".to_string()
            } else {
                format!("remote{r}")
            },
            url: None,
        });
    }
    repo.remotes = Loadable::Ready(Arc::new(remotes_vec.clone()));

    let mut remote = Vec::with_capacity(remote_branches);
    for ix in 0..remote_branches {
        let remote_name = if remotes <= 1 || ix % remotes == 0 {
            "origin".to_string()
        } else {
            format!("remote{}", ix % remotes)
        };
        remote.push(RemoteBranch {
            remote: remote_name,
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target: target.clone(),
        });
    }
    repo.remote_branches = Loadable::Ready(Arc::new(remote));

    let mut worktrees_vec = Vec::with_capacity(worktrees);
    for ix in 0..worktrees {
        let path = if ix == 0 {
            repo.spec.workdir.clone()
        } else {
            std::path::PathBuf::from(format!("/tmp/bench-worktree-{ix}"))
        };
        worktrees_vec.push(Worktree {
            path,
            head: Some(target.clone()),
            branch: Some(format!("feature/worktree/{ix}")),
            detached: ix % 7 == 0,
        });
    }
    repo.worktrees = Loadable::Ready(Arc::new(worktrees_vec));

    let mut submodules_vec = Vec::with_capacity(submodules);
    for ix in 0..submodules {
        submodules_vec.push(Submodule {
            path: std::path::PathBuf::from(format!("deps/submodule_{ix}")),
            head: CommitId(format!("{:040x}", 200_000usize.saturating_add(ix)).into()),
            status: if ix % 5 == 0 {
                SubmoduleStatus::HeadMismatch
            } else {
                SubmoduleStatus::UpToDate
            },
        });
    }
    repo.submodules = Loadable::Ready(Arc::new(submodules_vec));

    let stash_base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_100_000);
    let mut stashes_vec = Vec::with_capacity(stashes);
    for ix in 0..stashes {
        stashes_vec.push(StashEntry {
            index: ix,
            id: CommitId(format!("{:040x}", 300_000usize.saturating_add(ix)).into()),
            message: format!("WIP synthetic stash #{ix}").into(),
            created_at: Some(stash_base + Duration::from_secs(ix as u64)),
        });
    }
    repo.stashes = Loadable::Ready(Arc::new(stashes_vec));

    // Minimal "repo is open" status.
    repo.open = Loadable::Ready(());

    repo
}

pub(crate) fn build_repo_switch_repo_state(
    repo_id: RepoId,
    workdir: &str,
    commits: &[Commit],
    local_branches: usize,
    remote_branches: usize,
    remotes: usize,
    status_entries: usize,
    diff_path: Option<&str>,
) -> RepoState {
    let mut repo =
        build_synthetic_repo_state(local_branches, remote_branches, remotes, 0, 0, 24, commits);
    repo.id = repo_id;
    repo.spec = RepoSpec {
        workdir: std::path::PathBuf::from(workdir),
    };

    let log_page = Arc::new(LogPage {
        commits: commits.iter().take(200).cloned().collect(),
        next_cursor: commits.get(200).map(|commit| LogCursor {
            last_seen: commit.id.clone(),
            resume_from: None,
        }),
    });
    repo.history_state.log = Loadable::Ready(Arc::clone(&log_page));
    repo.history_state.log_rev = 1;
    repo.log = Loadable::Ready(log_page);
    repo.log_rev = 1;

    repo.status = Loadable::Ready(Arc::new(build_synthetic_repo_status(status_entries)));
    repo.status_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(build_tags_targeting_commits(commits, 32)));
    repo.tags_rev = 1;
    repo.remote_tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_tags_rev = 1;
    repo.rebase_in_progress = Loadable::Ready(false);
    repo.merge_commit_message = Loadable::Ready(None);
    repo.merge_message_rev = 1;
    repo.open_rev = 1;

    if let Some(selected_commit) = commits.first() {
        repo.history_state.selected_commit = Some(selected_commit.id.clone());
        repo.history_state.selected_commit_rev = 1;
        repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
            id: selected_commit.id.clone(),
            message: format!(
                "Synthetic selected commit for {}",
                repo.spec.workdir.display()
            ),
            committed_at: "2023-11-14 22:13".to_string(),
            parent_ids: selected_commit.parent_ids.to_vec(),
            files: (0..48)
                .map(|ix| CommitFileChange {
                    path: std::path::PathBuf::from(format!("src/module_{}/file_{ix}.rs", ix % 12)),
                    kind: if ix % 5 == 0 {
                        FileStatusKind::Added
                    } else {
                        FileStatusKind::Modified
                    },
                })
                .collect(),
        }));
        repo.history_state.commit_details_rev = 1;
    }

    if let Some(path) = diff_path {
        repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
            path: std::path::PathBuf::from(path),
            area: DiffArea::Unstaged,
        });
        repo.diff_state.diff_state_rev = 1;
        repo.diff_state.diff_rev = 1;
    }

    repo
}

/// Populate a repo's diff state with fully loaded diff content + file text,
/// simulating a user who has a file diff open and visible.
pub(crate) fn populate_loaded_diff_state(repo: &mut RepoState, path: &str, diff_line_count: usize) {
    let target = DiffTarget::WorkingTree {
        path: std::path::PathBuf::from(path),
        area: DiffArea::Unstaged,
    };
    repo.diff_state.diff = Loadable::Ready(Arc::new(Diff {
        target: target.clone(),
        lines: build_synthetic_diff_lines(diff_line_count),
    }));
    repo.diff_state.diff_file = Loadable::Ready(Some(Arc::new(FileDiffText::new(
        std::path::PathBuf::from(path),
        Some(build_synthetic_file_content(diff_line_count / 2)),
        Some(build_synthetic_file_content(
            diff_line_count / 2 + diff_line_count / 4,
        )),
    ))));
    repo.diff_state.diff_file_rev = 1;
}

/// Set the conflict state on a repo so the diff target is recognized as a
/// conflict path by the reducer.
pub(crate) fn populate_conflict_state(repo: &mut RepoState, path: &str, line_count: usize) {
    let path_buf = std::path::PathBuf::from(path);
    repo.conflict_state.conflict_file_path = Some(path_buf.clone());
    let content: Arc<str> = Arc::from(build_synthetic_file_content(line_count));
    repo.conflict_state.conflict_file = Loadable::Ready(Some(ConflictFile {
        path: path_buf.into(),
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: None,
        current_bytes: None,
        base: Some(Arc::clone(&content)),
        ours: Some(Arc::clone(&content)),
        theirs: Some(Arc::clone(&content)),
        current: Some(content),
    }));
    repo.conflict_state.conflict_rev = 1;
}

pub(crate) fn build_synthetic_diff_lines(count: usize) -> Vec<DiffLine> {
    let mut lines = Vec::with_capacity(count);
    lines.push(DiffLine {
        kind: DiffLineKind::Header,
        text: "diff --git a/src/main.rs b/src/main.rs".into(),
    });
    lines.push(DiffLine {
        kind: DiffLineKind::Header,
        text: "index abc1234..def5678 100644".into(),
    });

    let remaining = count.saturating_sub(2);
    let mut ix = 0;
    while ix < remaining {
        if ix % 50 == 0 {
            lines.push(DiffLine {
                kind: DiffLineKind::Hunk,
                text: format!(
                    "@@ -{0},{1} +{0},{1} @@ fn synthetic_function_{0}()",
                    ix + 1,
                    50.min(remaining - ix)
                )
                .into(),
            });
            ix += 1;
            if ix >= remaining {
                break;
            }
        }

        let kind = match ix % 7 {
            0..=3 => DiffLineKind::Context,
            4 | 5 => DiffLineKind::Add,
            _ => DiffLineKind::Remove,
        };
        lines.push(DiffLine {
            kind,
            text: format!("    let synthetic_var_{ix} = compute_value({ix}); // line {ix}").into(),
        });
        ix += 1;
    }

    lines
}

pub(crate) fn build_synthetic_file_content(line_count: usize) -> String {
    let mut content = String::with_capacity(line_count * 60);
    for ix in 0..line_count {
        content.push_str(&format!("    let line_{ix} = process_data({ix});\n"));
    }
    content
}

pub(crate) fn build_repo_switch_minimal_repo_state(repo_id: RepoId, workdir: &str) -> RepoState {
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: std::path::PathBuf::from(workdir),
        },
    );
    repo.open = Loadable::Ready(());
    repo.open_rev = 1;
    repo
}

pub(crate) fn build_synthetic_repo_status(entries: usize) -> RepoStatus {
    RepoStatus {
        staged: Vec::new(),
        unstaged: build_synthetic_status_entries(entries, DiffArea::Unstaged),
    }
}

pub(crate) fn build_synthetic_status_entries(entries: usize, area: DiffArea) -> Vec<FileStatus> {
    let mut items = Vec::with_capacity(entries);
    for ix in 0..entries {
        let (path, kind) = match area {
            DiffArea::Unstaged => (
                std::path::PathBuf::from(format!("src/{:02}/nested/path/file_{ix}.rs", ix % 24)),
                if ix % 11 == 0 {
                    FileStatusKind::Added
                } else {
                    FileStatusKind::Modified
                },
            ),
            DiffArea::Staged => (
                std::path::PathBuf::from(format!(
                    "release/{:02}/deploy/assets/staged_file_{ix}.toml",
                    ix % 32
                )),
                match ix % 13 {
                    0 => FileStatusKind::Deleted,
                    1 => FileStatusKind::Renamed,
                    2 => FileStatusKind::Added,
                    _ => FileStatusKind::Modified,
                },
            ),
        };

        items.push(FileStatus {
            path,
            kind,
            conflict: None,
        });
    }
    items
}

pub(crate) fn build_synthetic_partially_staged_entries(entries: usize) -> Vec<FileStatus> {
    let mut items = Vec::with_capacity(entries);
    for ix in 0..entries {
        items.push(FileStatus {
            path: std::path::PathBuf::from(format!(
                "src/{:02}/partially_staged/file_{ix:04}.rs",
                ix % 24
            )),
            kind: FileStatusKind::Modified,
            conflict: None,
        });
    }
    items
}

pub(crate) fn build_synthetic_status_entries_mixed_depth(entries: usize) -> Vec<FileStatus> {
    let mut items = Vec::with_capacity(entries);
    for ix in 0..entries {
        let mut path = match ix % 4 {
            0 => std::path::PathBuf::from("src"),
            1 => std::path::PathBuf::from("docs"),
            2 => std::path::PathBuf::from("assets"),
            _ => std::path::PathBuf::from("crates"),
        };
        let extra_depth = 2 + (ix % 12);
        for depth_ix in 0..extra_depth {
            path.push(format!(
                "segment_{depth_ix:02}_{}_{:03}",
                ix % 23,
                (ix.wrapping_mul(17).wrapping_add(depth_ix)) % 257
            ));
        }
        let extension = match ix % 5 {
            0 => "rs",
            1 => "toml",
            2 => "md",
            3 => "json",
            _ => "yaml",
        };
        path.push(format!("file_{ix:05}.{extension}"));

        let kind = match ix % 11 {
            0 => FileStatusKind::Added,
            1 => FileStatusKind::Deleted,
            2 => FileStatusKind::Renamed,
            3 => FileStatusKind::Conflicted,
            _ => FileStatusKind::Modified,
        };
        items.push(FileStatus {
            path,
            kind,
            conflict: None,
        });
    }
    items
}

pub(crate) fn build_synthetic_commits(count: usize) -> Vec<Commit> {
    build_synthetic_commits_with_merge_stride(count, 50, 40)
}

pub(crate) fn build_synthetic_commits_with_merge_stride(
    count: usize,
    merge_every: usize,
    merge_back_distance: usize,
) -> Vec<Commit> {
    if count == 0 {
        return Vec::new();
    }

    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let mut commits = Vec::with_capacity(count);

    for ix in 0..count {
        let id = CommitId(format!("{:040x}", ix).into());

        let mut parent_ids = Vec::new();
        if ix > 0 {
            parent_ids.push(CommitId(format!("{:040x}", ix - 1).into()));
        }
        // Synthetic merge-like commits at a fixed cadence.
        if merge_every > 0
            && merge_back_distance > 0
            && ix >= merge_back_distance
            && ix % merge_every == 0
        {
            parent_ids.push(CommitId(
                format!("{:040x}", ix - merge_back_distance).into(),
            ));
        }

        commits.push(Commit {
            id,
            parent_ids: parent_ids.into(),
            summary: format!("Commit {ix} - synthetic benchmark history entry").into(),
            author: format!("Author {}", ix % 10).into(),
            time: base + Duration::from_secs(ix as u64),
        });
    }

    // Most history/UI code expects log order: newest commit first, then older commits.
    // Returning the synthetic history in ascending order creates a pathological graph where
    // every commit appears to open a fresh lane before its parent is encountered.
    commits.reverse();
    commits
}

/// Build branches whose targets are spread across the commit list rather than
/// all pointing at the first commit, giving a realistic decoration-map workload.
pub(crate) fn build_branches_targeting_commits(
    commits: &[Commit],
    local_count: usize,
    remote_count: usize,
) -> (Vec<Branch>, Vec<RemoteBranch>) {
    let first_target = commits
        .first()
        .map(|c| c.id.clone())
        .unwrap_or_else(|| CommitId("0".repeat(40).into()));

    let mut branches = Vec::with_capacity(local_count.max(1));
    branches.push(Branch {
        name: "main".to_string(),
        target: first_target.clone(),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: "main".to_string(),
        }),
        divergence: Some(UpstreamDivergence {
            ahead: 1,
            behind: 2,
        }),
    });
    let commit_len = commits.len().max(1);
    for ix in 0..local_count.saturating_sub(1) {
        let target_ix = (ix.wrapping_mul(7)) % commit_len;
        let target = commits
            .get(target_ix)
            .map(|c| c.id.clone())
            .unwrap_or_else(|| first_target.clone());
        branches.push(Branch {
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target,
            upstream: None,
            divergence: None,
        });
    }

    let mut remote = Vec::with_capacity(remote_count);
    for ix in 0..remote_count {
        let target_ix = (ix.wrapping_mul(13)) % commit_len;
        let target = commits
            .get(target_ix)
            .map(|c| c.id.clone())
            .unwrap_or_else(|| first_target.clone());
        let remote_name = if ix % 4 == 0 {
            "origin".to_string()
        } else {
            format!("upstream{}", ix % 3)
        };
        remote.push(RemoteBranch {
            remote: remote_name,
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target,
        });
    }

    (branches, remote)
}

/// Build tags whose targets are spread across the commit list.
pub(crate) fn build_tags_targeting_commits(commits: &[Commit], count: usize) -> Vec<Tag> {
    let commit_len = commits.len().max(1);
    let mut tags = Vec::with_capacity(count);
    for ix in 0..count {
        let target_ix = (ix.wrapping_mul(11)) % commit_len;
        let target = commits
            .get(target_ix)
            .map(|c| c.id.clone())
            .unwrap_or_else(|| CommitId("0".repeat(40).into()));
        tags.push(Tag {
            name: format!("v{}.{}.{}", ix / 100, (ix / 10) % 10, ix % 10),
            target,
        });
    }
    tags
}

/// Build simple stash entries whose IDs do NOT match any commit in the log.
/// Use this for scenarios where stash entries exist but no stash-like commits
/// appear in the commit list (balanced scenario).
pub(crate) fn build_simple_stash_entries(count: usize) -> (Vec<StashEntry>, Vec<Commit>) {
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_200_000);
    let mut entries = Vec::with_capacity(count);
    for ix in 0..count {
        entries.push(StashEntry {
            index: ix,
            id: CommitId(format!("{:040x}", 500_000usize.saturating_add(ix)).into()),
            message: format!("On main: stash message {ix}").into(),
            created_at: Some(base + Duration::from_secs(ix as u64)),
        });
    }
    (entries, Vec::new())
}

/// Build stash entries with matching stash-like commits and their helper (index)
/// commits, injected into the log so the full stash filtering path fires.
pub(crate) fn build_stash_fixture_commits(
    base_commits: &[Commit],
    stash_count: usize,
    start_ix: usize,
) -> (Vec<StashEntry>, Vec<Commit>) {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_200_000);
    let base_len = base_commits.len().max(1);
    let mut stash_entries = Vec::with_capacity(stash_count);
    let mut extra_commits = Vec::with_capacity(stash_count * 2);

    for i in 0..stash_count {
        let parent_ix = i % base_len;
        let parent_id = base_commits
            .get(parent_ix)
            .map(|c| c.id.clone())
            .unwrap_or_else(|| CommitId(format!("{:040x}", 0).into()));

        // Stash helper (index commit) — secondary parent of the stash tip
        let helper_ix = start_ix + i * 2;
        let helper_id = CommitId(format!("{:040x}", helper_ix).into());
        extra_commits.push(Commit {
            id: helper_id.clone(),
            parent_ids: vec![parent_id.clone()].into(),
            summary: format!("index on main: {i}").into(),
            author: "Author 0".into(),
            time: base_time + Duration::from_secs(i as u64 * 2),
        });

        // Stash tip — 2 parents, stash-like summary
        let tip_ix = start_ix + i * 2 + 1;
        let tip_id = CommitId(format!("{:040x}", tip_ix).into());
        extra_commits.push(Commit {
            id: tip_id.clone(),
            parent_ids: vec![parent_id, helper_id].into(),
            summary: format!("WIP on main: stash message {i}").into(),
            author: "Author 0".into(),
            time: base_time + Duration::from_secs(i as u64 * 2 + 1),
        });

        stash_entries.push(StashEntry {
            index: i,
            id: tip_id,
            message: format!("On main: stash message {i}").into(),
            created_at: Some(base_time + Duration::from_secs(i as u64 * 2 + 1)),
        });
    }

    (stash_entries, extra_commits)
}

pub(crate) fn build_synthetic_commit_details(files: usize, depth: usize) -> CommitDetails {
    build_synthetic_commit_details_with_message(
        files,
        depth,
        "Synthetic benchmark commit details message\n\nWith body.".to_string(),
    )
}

pub(crate) fn build_synthetic_commit_details_with_message(
    files: usize,
    depth: usize,
    message: String,
) -> CommitDetails {
    let id = CommitId("d".repeat(40).into());
    let mut out = Vec::with_capacity(files);
    for ix in 0..files {
        let kind = match ix % 23 {
            0 => FileStatusKind::Deleted,
            1 | 2 => FileStatusKind::Renamed,
            3..=5 => FileStatusKind::Added,
            6 => FileStatusKind::Conflicted,
            7 => FileStatusKind::Untracked,
            _ => FileStatusKind::Modified,
        };

        let mut path = std::path::PathBuf::new();
        let depth = depth.max(1);
        for d in 0..depth {
            path.push(format!("dir{}_{}", d, ix % 128));
        }
        path.push(format!("file_{ix}.rs"));

        out.push(CommitFileChange { path, kind });
    }

    CommitDetails {
        id,
        message,
        committed_at: "2024-01-01T00:00:00Z".to_string(),
        parent_ids: vec![CommitId("c".repeat(40).into())],
        files: out,
    }
}

/// Like `build_synthetic_commit_details` but with a different commit ID
/// (the `id_char` is repeated 40 times to form the ID hex string).
pub(crate) fn build_synthetic_commit_details_with_id(
    files: usize,
    depth: usize,
    id_char: &str,
) -> CommitDetails {
    let mut details = build_synthetic_commit_details(files, depth);
    details.id = CommitId(id_char.repeat(40).into());
    details.parent_ids = vec![CommitId("d".repeat(40).into())];
    details
}

/// Like `build_synthetic_commit_details` but every file path is globally unique
/// (no `ix % 128` clamping on directory names). This produces files that all
/// miss the path-display cache, triggering cache clears for lists > 8192.
pub(crate) fn build_synthetic_commit_details_unique_paths(
    files: usize,
    depth: usize,
) -> CommitDetails {
    let id = CommitId("f".repeat(40).into());
    let depth = depth.max(1);
    let mut out = Vec::with_capacity(files);
    for ix in 0..files {
        let kind = match ix % 23 {
            0 => FileStatusKind::Deleted,
            1 | 2 => FileStatusKind::Renamed,
            3..=5 => FileStatusKind::Added,
            6 => FileStatusKind::Conflicted,
            7 => FileStatusKind::Untracked,
            _ => FileStatusKind::Modified,
        };
        let mut path = std::path::PathBuf::new();
        for d in 0..depth {
            // Use (ix / 256) and (ix % 256) to spread across unique directory names.
            path.push(format!("dir{}_{}_{}", d, ix / 256, ix % 256));
        }
        path.push(format!("file_{ix}.rs"));
        out.push(CommitFileChange { path, kind });
    }
    CommitDetails {
        id,
        message: "Synthetic commit details with unique paths for cache churn benchmark".to_string(),
        committed_at: "2024-01-01T00:00:00Z".to_string(),
        parent_ids: vec![CommitId("d".repeat(40).into())],
        files: out,
    }
}

pub(crate) fn build_synthetic_commit_message(min_bytes: usize, line_bytes: usize) -> String {
    let min_bytes = min_bytes.max(1);
    let line_bytes = line_bytes.max(40);
    let mut message = String::from("Synthetic benchmark commit subject\n\n");
    let line_count = min_bytes
        .saturating_div(line_bytes.max(1))
        .saturating_add(16)
        .max(16);
    let body_lines = build_synthetic_source_lines(line_count, line_bytes);
    for (ix, line) in body_lines.iter().enumerate() {
        message.push_str(line.as_str());
        message.push('\n');
        if ix % 8 == 7 {
            message.push('\n');
        }
    }
    while message.len() < min_bytes {
        message.push_str("benchmark body filler line for commit details rendering coverage\n");
    }
    message
}

pub(crate) fn count_commit_message_lines(message: &str) -> usize {
    if message.is_empty() {
        1
    } else {
        message.lines().count().max(1)
    }
}

pub(crate) fn build_commit_details_message_render_state(
    message: &str,
    render: CommitDetailsMessageRenderConfig,
) -> CommitDetailsMessageRenderState {
    let snapshot = TextModel::from_large_text(message).snapshot();
    let wrap_columns = wrap_columns_for_benchmark_width(render.wrap_width_px);
    let mut shaped_bytes = 0usize;
    let mut visible_lines = Vec::with_capacity(render.visible_lines.max(1));
    for line in message.lines().take(render.visible_lines.max(1)) {
        let (shaping_hash, capped_len) =
            hash_text_input_shaping_slice(line, render.max_shape_bytes.max(1));
        visible_lines.push(CommitDetailsVisibleMessageLine {
            shaping_hash,
            capped_len,
            wrap_rows: estimate_tabbed_wrap_rows(line, wrap_columns),
        });
        shaped_bytes = shaped_bytes.saturating_add(capped_len);
    }

    CommitDetailsMessageRenderState {
        message_len: snapshot.len(),
        line_count: snapshot.shared_line_starts().len(),
        shaped_bytes,
        visible_lines,
    }
}

pub(crate) fn measure_commit_message_visible_window(
    render: Option<&CommitDetailsMessageRenderState>,
) -> (usize, usize) {
    let Some(render) = render else {
        return (0, 0);
    };

    (render.visible_lines.len(), render.shaped_bytes)
}

pub(crate) fn commit_details_message_hash(
    message_len: usize,
    render: Option<&CommitDetailsMessageRenderState>,
    hasher: &mut FxHasher,
) {
    let Some(render) = render else {
        message_len.hash(hasher);
        return;
    };

    render.message_len.hash(hasher);
    render.line_count.hash(hasher);

    for line in &render.visible_lines {
        line.shaping_hash.hash(hasher);
        line.capped_len.hash(hasher);
        line.wrap_rows.hash(hasher);
    }

    render.visible_lines.len().hash(hasher);
    render.shaped_bytes.hash(hasher);
}

pub(crate) fn hash_shared_string_identity(label: &SharedString, hasher: &mut FxHasher) {
    let text = label.as_ref();
    text.as_ptr().hash(hasher);
    text.len().hash(hasher);
}

pub(crate) fn hash_path_identity(path: &std::path::Path, hasher: &mut FxHasher) {
    let text = path.as_os_str().as_encoded_bytes();
    text.as_ptr().hash(hasher);
    text.len().hash(hasher);
}

pub(crate) fn hash_optional_path_identity(path: Option<&std::path::Path>, hasher: &mut FxHasher) {
    match path {
        Some(path) => {
            true.hash(hasher);
            hash_path_identity(path, hasher);
        }
        None => false.hash(hasher),
    }
}

pub(crate) fn hash_status_multi_selection_path_sample(
    paths: &[std::path::PathBuf],
    hasher: &mut FxHasher,
) {
    let len = paths.len();
    len.hash(hasher);
    for path in paths.iter().take(4) {
        hash_path_identity(path.as_path(), hasher);
    }
    if len > 4 {
        for path in paths.iter().rev().take(4) {
            hash_path_identity(path.as_path(), hasher);
        }
    }
}

pub(crate) fn commit_details_cached_row_hash(
    details: &CommitDetails,
    message_render: Option<&CommitDetailsMessageRenderState>,
    file_rows: &mut crate::view::rows::CommitFileRowPresentationCache<CommitId>,
) -> u64 {
    let mut h = FxHasher::default();
    details.id.as_ref().hash(&mut h);
    commit_details_message_hash(details.message.len(), message_render, &mut h);
    let rows = file_rows.rows_for(&details.id, &details.files);
    hash_commit_file_row_presentations(rows.as_ref()).hash(&mut h);
    details.files.len().hash(&mut h);
    h.finish()
}

pub(crate) fn hash_commit_file_row_presentations(
    rows: &[crate::view::rows::CommitFileRowPresentation],
) -> u64 {
    let mut hasher = FxHasher::default();
    rows.len().hash(&mut hasher);
    for row in rows {
        row.visuals.kind_key.hash(&mut hasher);
        let label = row.label.as_ref();
        label.as_ptr().hash(&mut hasher);
        label.len().hash(&mut hasher);
    }
    hasher.finish()
}

pub(crate) fn build_bench_file_diff_rebuild_from_text(
    path: impl Into<std::path::PathBuf>,
    old: &str,
    new: &str,
) -> (
    Arc<crate::view::panes::main::diff_cache::PagedFileDiffRows>,
    Arc<crate::view::panes::main::diff_cache::PagedFileDiffInlineRows>,
) {
    let file = FileDiffText::new(path.into(), Some(old.to_owned()), Some(new.to_owned()));
    let rebuild = crate::view::panes::main::diff_cache::build_file_diff_cache_rebuild(
        &file,
        Path::new("/tmp/gitcomet-bench"),
    );
    (rebuild.row_provider, rebuild.inline_row_provider)
}

pub(crate) fn build_synthetic_source_lines(count: usize, target_line_bytes: usize) -> Vec<String> {
    let target_line_bytes = target_line_bytes.max(32);
    let mut lines = Vec::with_capacity(count);
    for ix in 0..count {
        let indent = " ".repeat((ix % 8) * 2);
        let mut line = match ix % 10 {
            0 => format!("{indent}fn func_{ix}(x: usize) -> usize {{ x + {ix} }}"),
            1 => format!("{indent}let value_{ix} = \"string {ix}\";"),
            2 => format!("{indent}// comment {ix} with some extra words and tokens"),
            3 => format!("{indent}if value_{ix} > 10 {{ return value_{ix}; }}"),
            4 => format!(
                "{indent}for i in 0..{r} {{ sum += i; }}",
                r = (ix % 100) + 1
            ),
            5 => format!("{indent}match tag_{ix} {{ Some(v) => v, None => 0 }}"),
            6 => format!("{indent}struct S{ix} {{ a: i32, b: String }}"),
            7 => format!(
                "{indent}impl S{ix} {{ fn new() -> Self {{ Self {{ a: 0, b: String::new() }} }} }}"
            ),
            8 => format!("{indent}const CONST_{ix}: u64 = {v};", v = ix as u64 * 31),
            _ => format!("{indent}println!(\"{ix} {{}}\", value_{ix});"),
        };
        if line.len() < target_line_bytes {
            line.push(' ');
            line.push_str("//");
            while line.len() < target_line_bytes {
                line.push_str(" token_");
                line.push_str(&(ix % 997).to_string());
            }
        }
        lines.push(line);
    }
    lines
}

pub(crate) fn hash_file_diff_plan(plan: &gitcomet_core::file_diff::FileDiffPlan) -> u64 {
    let mut h = FxHasher::default();
    plan.row_count.hash(&mut h);
    plan.inline_row_count.hash(&mut h);
    match plan.eof_newline {
        Some(gitcomet_core::file_diff::FileDiffEofNewline::MissingInOld) => 1u8,
        Some(gitcomet_core::file_diff::FileDiffEofNewline::MissingInNew) => 2u8,
        None => 0u8,
    }
    .hash(&mut h);
    plan.runs.len().hash(&mut h);
    for run in plan.runs.iter().take(256) {
        std::mem::discriminant(run).hash(&mut h);
        match run {
            gitcomet_core::file_diff::FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            } => {
                old_start.hash(&mut h);
                new_start.hash(&mut h);
                len.hash(&mut h);
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, len } => {
                old_start.hash(&mut h);
                len.hash(&mut h);
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, len } => {
                new_start.hash(&mut h);
                len.hash(&mut h);
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                old_start.hash(&mut h);
                new_start.hash(&mut h);
                len.hash(&mut h);
            }
        }
    }
    h.finish()
}

pub(crate) fn build_synthetic_replacement_alignment_documents(
    blocks: usize,
    old_block_lines: usize,
    new_block_lines: usize,
    context_lines: usize,
    target_line_bytes: usize,
) -> (String, String) {
    let blocks = blocks.max(1);
    let old_block_lines = old_block_lines.max(1);
    let new_block_lines = new_block_lines.max(1);
    let context_lines = context_lines.max(1);
    let target_line_bytes = target_line_bytes.max(80);

    let mut old_lines = Vec::new();
    let mut new_lines = Vec::new();
    old_lines.push("fn replacement_alignment_fixture() {".to_string());
    new_lines.push("fn replacement_alignment_fixture() {".to_string());

    for block_ix in 0..blocks {
        for context_ix in 0..context_lines {
            let line =
                build_synthetic_replacement_context_line(block_ix, context_ix, target_line_bytes);
            old_lines.push(line.clone());
            new_lines.push(line);
        }

        for line_ix in 0..old_block_lines {
            old_lines.push(build_synthetic_replacement_change_line(
                block_ix,
                line_ix,
                old_block_lines,
                target_line_bytes,
                "before",
            ));
        }
        for line_ix in 0..new_block_lines {
            new_lines.push(build_synthetic_replacement_change_line(
                block_ix,
                line_ix,
                new_block_lines,
                target_line_bytes,
                "after",
            ));
        }
    }

    old_lines.push("}".to_string());
    new_lines.push("}".to_string());

    let mut old_text = old_lines.join("\n");
    old_text.push('\n');
    let mut new_text = new_lines.join("\n");
    new_text.push('\n');
    (old_text, new_text)
}

pub(crate) fn build_synthetic_replacement_context_line(
    block_ix: usize,
    context_ix: usize,
    target_line_bytes: usize,
) -> String {
    let mut line = format!(
        "    let context_{block_ix:03}_{context_ix:03} = stable_anchor(block_{block_ix:03}, {context_ix});"
    );
    if line.len() < target_line_bytes {
        line.push(' ');
        line.push_str("//");
        while line.len() < target_line_bytes {
            line.push_str(" keep_anchor");
        }
    }
    line
}

pub(crate) fn build_synthetic_replacement_change_line(
    block_ix: usize,
    line_ix: usize,
    block_lines: usize,
    target_line_bytes: usize,
    variant: &str,
) -> String {
    let logical_span = block_lines.max(1);
    let rotated_ix = (line_ix + (block_ix % 7) + 1) % logical_span;
    let logical_ix = if variant == "before" {
        line_ix
    } else {
        rotated_ix
    };

    let mut line = format!(
        "    let block_{block_ix:03}_slot_{logical_ix:03} = reconcile_entry(namespace::{variant}_source_{logical_ix:03}, synth_payload(block_{block_ix:03}, {logical_ix}), \"shared-payload-{block_ix:03}-{logical_ix:03}\");"
    );
    if line.len() < target_line_bytes {
        line.push(' ');
        line.push_str("//");
        while line.len() < target_line_bytes {
            if variant == "before" {
                line.push_str(" before_token");
            } else {
                line.push_str(" after_token");
            }
        }
    }
    line
}

pub(crate) fn line_starts_for_text(text: &str) -> Vec<usize> {
    let mut line_starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
    line_starts.push(0);
    for newline_ix in memchr::memchr_iter(b'\n', text.as_bytes()) {
        line_starts.push(newline_ix.saturating_add(1));
    }
    line_starts
}

pub(crate) fn bench_counter_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

pub(crate) fn build_text_input_streamed_highlights(
    text: &str,
    line_starts: &[usize],
    density: TextInputHighlightDensity,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let theme = AppTheme::gitcomet_dark();
    let style_primary = gpui::HighlightStyle {
        color: Some(theme.colors.accent.into()),
        ..gpui::HighlightStyle::default()
    };
    let style_secondary = gpui::HighlightStyle {
        color: Some(theme.colors.warning.into()),
        ..gpui::HighlightStyle::default()
    };
    let style_overlay = gpui::HighlightStyle {
        color: Some(theme.colors.success.into()),
        ..gpui::HighlightStyle::default()
    };

    let mut highlights = Vec::new();
    for line_ix in 0..line_starts.len() {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let mut line_end = line_starts.get(line_ix + 1).copied().unwrap_or(text.len());
        if line_end > line_start && text.as_bytes().get(line_end - 1) == Some(&b'\n') {
            line_end = line_end.saturating_sub(1);
        }
        if line_end <= line_start {
            continue;
        }
        let line_len = line_end.saturating_sub(line_start);

        match density {
            TextInputHighlightDensity::Dense => {
                let mut local = 0usize;
                while local + 2 < line_len {
                    let start = line_start + local;
                    let end = (start + 20).min(line_end);
                    if start < end {
                        let style = if local.is_multiple_of(24) {
                            style_primary
                        } else {
                            style_secondary
                        };
                        highlights.push((start..end, style));
                    }

                    let overlap_start = start.saturating_add(4).min(line_end);
                    let overlap_end = (overlap_start + 14).min(line_end);
                    if overlap_start < overlap_end {
                        highlights.push((overlap_start..overlap_end, style_overlay));
                    }

                    local = local.saturating_add(12);
                }
            }
            TextInputHighlightDensity::Sparse => {
                if line_ix % 8 == 0 {
                    let start = line_start.saturating_add(2).min(line_end);
                    let end = (start + 26).min(line_end);
                    if start < end {
                        highlights.push((start..end, style_primary));
                    }
                }
                if line_ix % 24 == 0 {
                    let start = line_start.saturating_add(10).min(line_end);
                    let end = (start + 18).min(line_end);
                    if start < end {
                        highlights.push((start..end, style_overlay));
                    }
                }
            }
        }
    }

    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    highlights
}
