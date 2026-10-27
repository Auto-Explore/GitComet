use super::super::auth::{
    clear_staged_git_auth_env, detect_auth_prompt_kind, detect_auth_prompt_kind_from_message,
    stage_git_auth_env,
};
use super::*;
use crate::model::{AppNotificationKind, DiagnosticKind};
use crate::msg::RepoCommandKind;
use gitcomet_core::auth::SSH_PASSPHRASE_PROMPT_MARKER;
use gitcomet_core::domain::{CommitId, DiffArea, DiffTarget, RepoSpec};
use gitcomet_core::error::{GitFailure, GitFailureId};
use gitcomet_core::services::{PullMode, RemoteUrlKind, ResetMode};
use std::path::Path;

fn repo_state(id: u64) -> RepoState {
    RepoState::new_opening(
        RepoId(id),
        RepoSpec {
            workdir: PathBuf::from("/tmp/gitcomet-state-util-tests"),
        },
    )
}

fn command_output(command: &str, stdout: &str, stderr: &str) -> CommandOutput {
    CommandOutput {
        command: command.to_string(),
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        exit_code: Some(0),
    }
}

fn dummy_log_entry(ix: usize) -> CommandLogEntry {
    CommandLogEntry {
        kind: crate::model::CommandLogKind::Other,
        time: SystemTime::UNIX_EPOCH,
        ok: true,
        command: format!("cmd-{ix}"),
        summary: String::new(),
        stdout: Arc::from(""),
        stderr: Arc::from(""),
        announce_success: true,
        handled_by_auth_prompt: false,
        failure_id: None,
        hook_operation_id: None,
    }
}

#[test]
fn diff_reload_effects_cover_image_svg_and_non_file_targets() {
    let repo_id = RepoId(7);
    let repo_state = repo_state(repo_id.0);
    let png = DiffTarget::working_tree(PathBuf::from("img.PNG"), DiffArea::Unstaged);
    let png_effects = diff_reload_effects(
        &crate::store::reducer::test_inputs::for_target(&repo_state, &png.clone()),
        &repo_state,
        repo_id,
        png.clone(),
    );
    assert!(diff_target_wants_image_preview(&png));
    assert!(!diff_target_is_svg(&png));
    assert_eq!(png_effects.len(), 2);
    assert!(matches!(png_effects[0], Effect::LoadDiff { .. }));
    assert!(matches!(png_effects[1], Effect::LoadDiffFileImage { .. }));

    let svg = DiffTarget::working_tree(PathBuf::from("diagram.svg"), DiffArea::Unstaged);
    let svg_effects = diff_reload_effects(
        &crate::store::reducer::test_inputs::for_target(&repo_state, &svg.clone()),
        &repo_state,
        repo_id,
        svg.clone(),
    );
    assert!(diff_target_wants_image_preview(&svg));
    assert!(diff_target_is_svg(&svg));
    assert_eq!(svg_effects.len(), 3);
    assert!(matches!(svg_effects[2], Effect::LoadDiffFile { .. }));

    let text_no_ext = DiffTarget::working_tree(PathBuf::from("README"), DiffArea::Unstaged);
    assert!(!diff_target_wants_image_preview(&text_no_ext));
    assert_eq!(
        diff_reload_effects(
            &crate::store::reducer::test_inputs::for_target(&repo_state, &text_no_ext),
            &repo_state,
            repo_id,
            text_no_ext
        )
        .len(),
        2
    );

    let range_without_path = DiffTarget::commit_range(
        CommitId("abc123".into()),
        Some(CommitId("def456".into())),
        None,
    );
    assert!(!diff_target_wants_image_preview(&range_without_path));
    assert!(!diff_target_is_svg(&range_without_path));
    assert_eq!(
        diff_reload_effects(
            &crate::store::reducer::test_inputs::for_target(&repo_state, &range_without_path),
            &repo_state,
            repo_id,
            range_without_path
        )
        .len(),
        0
    );
}

#[test]
fn duplicate_worktree_paths_preserve_first_submodule_and_first_eligible_preview() {
    use gitcomet_core::domain::{DiffPreviewTextSide, FileStatus, RepoStatus};

    let directory = tempfile::tempdir().unwrap();
    for (kinds, submodule, preview_side) in [
        (
            vec![FileStatusKind::Modified, FileStatusKind::Added],
            false,
            Some(DiffPreviewTextSide::New),
        ),
        (
            vec![
                FileStatusKind::Modified,
                FileStatusKind::Deleted,
                FileStatusKind::Added,
            ],
            false,
            Some(DiffPreviewTextSide::Old),
        ),
        (
            vec![FileStatusKind::Renamed, FileStatusKind::Untracked],
            false,
            Some(DiffPreviewTextSide::New),
        ),
        (
            vec![FileStatusKind::Untracked, FileStatusKind::Deleted],
            false,
            Some(DiffPreviewTextSide::New),
        ),
        (
            vec![FileStatusKind::Deleted, FileStatusKind::Added],
            true,
            None,
        ),
    ] {
        let mut repo = repo_state(12);
        repo.spec.workdir = directory.path().to_path_buf();
        let path = PathBuf::from("selected.rs");
        repo.head_gitlink_paths.insert(path.clone());
        repo.set_status(Loadable::Ready(Arc::new(RepoStatus {
            unstaged: Arc::new(
                kinds
                    .iter()
                    .map(|&kind| FileStatus {
                        path: path.clone(),
                        kind,
                        conflict: None,
                    })
                    .collect(),
            ),
            staged: Arc::default(),
        })));
        let target = DiffTarget::working_tree(path, DiffArea::Unstaged);
        let plan = selected_diff_load_plan(
            &crate::store::reducer::test_inputs::for_target(&repo, &target),
            &repo,
            &target,
        );
        assert_eq!(
            plan,
            SelectedDiffLoadPlan {
                load_submodule_summary: submodule,
                preview_text_side: preview_side,
                ..SelectedDiffLoadPlan::default()
            },
            "status order: {kinds:?}"
        );
    }
}

#[test]
fn content_preview_forces_full_content_preview_plan() {
    let mut repo = repo_state(9);
    repo.diff_state.content_preview = true;

    // Working-tree content is read from disk: no patch diff, no file text, no
    // preview-text-file load.
    let worktree = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
    let plan = selected_diff_load_plan(
        &crate::store::reducer::test_inputs::for_target(&repo, &worktree),
        &repo,
        &worktree,
    );
    assert!(!plan.load_patch_diff);
    assert!(!plan.load_file_text);
    assert_eq!(plan.preview_text_side, None);
    assert!(!plan.load_file_image);

    // Commit content reads the New-side blob via a preview text file.
    let commit = DiffTarget::commit(CommitId("abc123".into()), PathBuf::from("src/lib.rs"));
    let plan = selected_diff_load_plan(
        &crate::store::reducer::test_inputs::for_target(&repo, &commit),
        &repo,
        &commit,
    );
    assert!(!plan.load_patch_diff);
    assert_eq!(
        plan.preview_text_side,
        Some(gitcomet_core::domain::DiffPreviewTextSide::New)
    );

    // An image is still loaded as an image, not as text.
    let image = DiffTarget::commit(CommitId("abc123".into()), PathBuf::from("logo.png"));
    let plan = selected_diff_load_plan(
        &crate::store::reducer::test_inputs::for_target(&repo, &image),
        &repo,
        &image,
    );
    assert!(plan.load_file_image);
    assert_eq!(plan.preview_text_side, None);

    // Without the flag, a tracked file still gets a normal patch diff.
    repo.diff_state.content_preview = false;
    assert!(
        selected_diff_load_plan(
            &crate::store::reducer::test_inputs::for_target(&repo, &worktree),
            &repo,
            &worktree
        )
        .load_patch_diff
    );
}

#[test]
fn preview_only_svg_still_loads_file_text_for_the_code_view() {
    use crate::model::Shared;
    use gitcomet_core::domain::{FileStatus, RepoStatus};

    let mut repo = repo_state(11);
    let svg_path = PathBuf::from("assets/diagram.svg");
    let png_path = PathBuf::from("assets/logo.png");
    repo.status = Loadable::Ready(Shared::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![
            FileStatus {
                path: svg_path.clone(),
                kind: FileStatusKind::Untracked,
                conflict: None,
            },
            FileStatus {
                path: png_path.clone(),
                kind: FileStatusKind::Untracked,
                conflict: None,
            },
        ]),
        staged: std::sync::Arc::new(vec![]),
    }));

    // An untracked SVG has no patch, but its source still has to load: the
    // Code view is the only place an SVG's text is ever shown.
    let svg = DiffTarget::working_tree(svg_path, DiffArea::Unstaged);
    let plan = selected_diff_load_plan(
        &crate::store::reducer::test_inputs::for_target(&repo, &svg),
        &repo,
        &svg,
    );
    assert!(!plan.load_patch_diff);
    assert!(plan.load_file_text);
    assert!(plan.load_file_image);
    // Image + preview text + file text is the widest SVG fan-out; it has to
    // stay inside the reload cap that `diff_reload_effect_count` asserts.
    assert!(plan.preview_text_side.is_some());
    assert_eq!(
        diff_reload_effect_count(
            &crate::store::reducer::test_inputs::for_target(&repo, &svg),
            &repo,
            &svg
        ),
        3
    );

    // A non-SVG image has no text view at all.
    let png = DiffTarget::working_tree(png_path, DiffArea::Unstaged);
    let plan = selected_diff_load_plan(
        &crate::store::reducer::test_inputs::for_target(&repo, &png),
        &repo,
        &png,
    );
    assert!(!plan.load_patch_diff);
    assert!(!plan.load_file_text);
    assert!(plan.load_file_image);

    // Content preview does not suppress the SVG file text either.
    repo.diff_state.content_preview = true;
    assert!(
        selected_diff_load_plan(
            &crate::store::reducer::test_inputs::for_target(&repo, &svg),
            &repo,
            &svg
        )
        .load_file_text
    );
}

#[test]
fn refresh_effects_request_expected_loads_and_reset_log_loading_more() {
    let mut primary = repo_state(1);
    primary.set_log_loading_more(true);
    let primary_effects = refresh_primary_effects(&mut primary);
    assert_eq!(primary_effects.len(), 5);
    assert!(!primary.log_loading_more);
    assert!(matches!(
        primary_effects[0],
        Effect::References(ReferenceEffect::LoadHeadBranch { .. })
    ));
    assert!(
        primary_effects
            .iter()
            .any(|effect| matches!(effect, Effect::Status(StatusEffect::LoadStatus { .. })))
    );
    // Counts wait for the status snapshot, including on the batch path.
    assert!(
        !primary_effects.iter().any(|effect| matches!(
            effect,
            Effect::Status(StatusEffect::LoadUncommittedLineStats { .. })
        )),
        "counts must not launch a second worktree walk"
    );
    assert!(matches!(
        primary_effects[4],
        Effect::LoadLog {
            limit: DEFAULT_LOG_PAGE_SIZE,
            ..
        }
    ));
    assert!(
        !primary_effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::Status(StatusEffect::LoadWorktreeStatus { .. })
                    | Effect::Status(StatusEffect::LoadStagedStatus { .. })
            )
        }),
        "primary refresh should coalesce staged and worktree status into LoadStatus"
    );
    assert!(
        primary_effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadRebaseAndMergeState { .. }))
    );

    let mut full = repo_state(2);
    full.set_log_loading_more(true);
    let full_effects = refresh_full_effects(&mut full, GitLogSettings::default());
    assert_eq!(full_effects.len(), 8);
    assert!(!full.log_loading_more);
    assert!(
        full_effects
            .iter()
            .any(|effect| matches!(effect, Effect::Status(StatusEffect::LoadStatus { .. })))
    );
    assert!(!full_effects.iter().any(|effect| matches!(
        effect,
        Effect::Status(StatusEffect::LoadUncommittedLineStats { .. })
    )));
    assert!(
        !full_effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::Status(StatusEffect::LoadWorktreeStatus { .. })
                    | Effect::Status(StatusEffect::LoadStagedStatus { .. })
            )
        }),
        "full refresh should coalesce staged and worktree status into LoadStatus"
    );
    assert!(
        !full_effects
            .iter()
            .any(|effect| matches!(effect, Effect::References(ReferenceEffect::LoadTags { .. }))),
        "tags should lazy-load by default instead of refresh_full_effects"
    );
    assert!(
        !full_effects.iter().any(|effect| matches!(
            effect,
            Effect::References(ReferenceEffect::LoadRemoteTags { .. })
        )),
        "remote tags should lazy-load from tag-specific UI instead of refresh_full_effects"
    );
    assert!(
        !full_effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadStashes { .. })),
        "stashes should now lazy-load from the sidebar instead of refresh_full_effects"
    );
    assert!(
        full_effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadRebaseAndMergeState { .. }))
    );

    let mut metadata = repo_state(3);
    metadata.set_open(Loadable::Ready(()));
    let mut metadata_effects = Vec::new();
    append_auto_background_metadata_effects(
        &mut metadata,
        GitLogSettings::default(),
        &mut metadata_effects,
    );
    assert!(
        metadata_effects
            .iter()
            .any(|effect| matches!(effect, Effect::References(ReferenceEffect::LoadTags { .. }))),
        "auto-idle metadata should request LoadTags"
    );
    assert!(
        metadata_effects.iter().any(|effect| matches!(
            effect,
            Effect::References(ReferenceEffect::LoadRemoteTags { .. })
        )),
        "auto-idle metadata should request LoadRemoteTags"
    );
    assert!(
        metadata_effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadSubmodules { .. })),
        "auto-idle metadata should request LoadSubmodules"
    );
}

#[test]
fn dedup_and_normalize_path_cover_duplicate_and_relative_branches() {
    let deduped = dedup_paths_in_order(vec![
        PathBuf::from("a"),
        PathBuf::from("b"),
        PathBuf::from("a"),
    ]);
    assert_eq!(deduped, vec![PathBuf::from("a"), PathBuf::from("b")]);

    let normalized = normalize_repo_path(PathBuf::from("."));
    assert!(normalized.is_absolute());
}

#[test]
fn push_notification_and_diagnostic_count_every_entry_without_a_cap() {
    let mut state = AppState::test_default();
    for ix in 0..205 {
        push_notification(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            AppNotificationKind::Info,
            format!("notification-{ix}"),
        );
    }
    assert_eq!(state.notifications.len(), 205);
    assert_eq!(state.notifications_seq, 205);
    assert_eq!(state.notifications[0].message, "notification-0");

    let mut repo = repo_state(3);
    for ix in 0..205 {
        push_diagnostic(
            &crate::store::reducer::test_inputs::clock(),
            &mut repo,
            DiagnosticKind::Info,
            format!("diagnostic-{ix}"),
        );
    }
    assert_eq!(repo.feedback.diagnostics.len(), 205);
    assert_eq!(repo.feedback.diagnostics_seq, 205);
    assert_eq!(repo.feedback.diagnostics[0].message, "diagnostic-0");
}

#[test]
fn command_and_action_logs_use_expected_stderr_and_keep_history() {
    let mut repo = repo_state(4);
    repo.feedback_mut().command_log = (0..200).map(dummy_log_entry).collect();
    push_command_log(
        &crate::store::reducer::test_inputs::clock(),
        &mut repo,
        true,
        &RepoCommandKind::FetchAll,
        &command_output("git fetch", "", "stderr from git"),
        None,
        false,
    );
    assert_eq!(repo.feedback.command_log.len(), 201);
    assert_eq!(repo.feedback.command_log_seq, 1);
    assert_eq!(repo.feedback.command_log[0].command, "cmd-0");
    assert_eq!(
        repo.feedback
            .command_log
            .last()
            .expect("last command log entry")
            .stderr,
        "stderr from git".into()
    );

    repo.feedback_mut().command_log = (0..200).map(dummy_log_entry).collect();
    push_action_log(
        &crate::store::reducer::test_inputs::clock(),
        &mut repo,
        false,
        "manual action".to_string(),
        "action failed".to_string(),
        Some(&Error::new(ErrorKind::Backend(
            "backend failure".to_string(),
        ))),
        false,
    );
    assert_eq!(repo.feedback.command_log.len(), 201);
    assert_eq!(repo.feedback.command_log_seq, 2);
    assert_eq!(repo.feedback.command_log[0].command, "cmd-0");
}

#[test]
fn conflict_autosolve_summary_covers_mode_and_detail_variants() {
    let history_summary = conflict_autosolve_telemetry_summary(
        ConflictAutosolveMode::History,
        Some(Path::new("conflict.txt")),
        6,
        3,
        4,
        1,
        ConflictAutosolveStats {
            history: 2,
            ..ConflictAutosolveStats::default()
        },
    );
    assert!(history_summary.contains("(history)"));
    assert!(history_summary.contains("history=2"));
    assert!(history_summary.contains("in conflict.txt"));

    let safe_summary = conflict_autosolve_telemetry_summary(
        ConflictAutosolveMode::Safe,
        None,
        1,
        1,
        1,
        1,
        ConflictAutosolveStats::default(),
    );
    assert!(safe_summary.contains("(safe)"));
    assert!(safe_summary.contains("details=none"));
}

#[test]
fn summarize_command_failure_covers_error_labels() {
    let failing_cases = vec![
        (RepoCommandKind::FetchAll, "Fetch"),
        (
            RepoCommandKind::PruneMergedBranches,
            "Prune merged branches",
        ),
        (RepoCommandKind::PruneLocalTags, "Prune local tags"),
        (
            RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            "Pull",
        ),
        (
            RepoCommandKind::PullBranch {
                remote: "origin".into(),
                branch: "main".into(),
            },
            "Pull",
        ),
        (
            RepoCommandKind::MergeRef {
                reference: "feature".into(),
            },
            "Merge",
        ),
        (
            RepoCommandKind::SquashRef {
                reference: "feature".into(),
            },
            "Squash",
        ),
        (RepoCommandKind::Push, "Push"),
        (RepoCommandKind::ForcePush, "Force push"),
        (
            RepoCommandKind::PushSetUpstream {
                remote: "origin".into(),
                branch: "main".into(),
            },
            "Push",
        ),
        (
            RepoCommandKind::SetUpstreamBranch {
                branch: "main".into(),
                upstream: Upstream {
                    remote: "origin".into(),
                    branch: "main".into(),
                },
            },
            "Set as tracking upstream",
        ),
        (
            RepoCommandKind::UnsetUpstreamBranch {
                branch: "main".into(),
            },
            "Unlink upstream branch",
        ),
        (
            RepoCommandKind::DeleteRemoteBranch {
                remote: "origin".into(),
                branch: "old".into(),
            },
            "Delete remote branch",
        ),
        (
            RepoCommandKind::DeleteRemoteBranches {
                remote: "origin".into(),
                branches: vec!["feat/a".into(), "feat/b".into()],
            },
            "Delete remote branches",
        ),
        (
            RepoCommandKind::PushTag {
                remote: "origin".into(),
                name: "v1".into(),
            },
            "Push tag",
        ),
        (
            RepoCommandKind::DeleteRemoteTag {
                remote: "origin".into(),
                name: "v1".into(),
            },
            "Delete remote tag",
        ),
        (
            RepoCommandKind::Reset {
                mode: ResetMode::Hard,
                target: "HEAD~1".into(),
            },
            "Reset",
        ),
        (
            RepoCommandKind::Rebase {
                onto: "main".into(),
            },
            "Rebase",
        ),
        (RepoCommandKind::RebaseContinue, "Rebase"),
        (RepoCommandKind::RebaseAbort, "Rebase"),
        (
            RepoCommandKind::InteractiveRebase {
                base: "HEAD~3".into(),
                interactive: true,
            },
            "Interactive rebase",
        ),
        (RepoCommandKind::MergeAbort, "Merge"),
        (
            RepoCommandKind::ApplyFileChange {
                commit_retry: None,
                target: gitcomet_core::domain::ApplyChangeTarget::commit(
                    CommitId("abcdef1234567890".into()),
                    PathBuf::from("a.txt"),
                ),
                commit: false,
            },
            "Apply change",
        ),
        (
            RepoCommandKind::CreateTag {
                name: "v2".into(),
                target: "HEAD".into(),
                message: None,
                annotated: false,
            },
            "Tag",
        ),
        (RepoCommandKind::DeleteTag { name: "v2".into() }, "Tag"),
        (
            RepoCommandKind::AddRemote {
                name: "origin".into(),
                url: "https://example.com/repo.git".into(),
            },
            "Remote",
        ),
        (
            RepoCommandKind::RemoveRemote {
                name: "origin".into(),
            },
            "Remote",
        ),
        (
            RepoCommandKind::SetRemoteUrl {
                name: "origin".into(),
                url: "https://example.com/repo.git".into(),
                kind: RemoteUrlKind::Fetch,
            },
            "Remote",
        ),
    ];

    for (command, label) in failing_cases {
        let (rendered_command, summary) =
            summarize_command(&command, &CommandOutput::default(), false, None);
        assert_eq!(rendered_command, label);
        assert_eq!(summary, format!("{label} failed"));
    }
}

#[test]
fn maintenance_summary_reports_what_the_run_did() {
    // Since git 2.54 the default strategy prints nothing over a pipe, even
    // after a full repack.
    let (_, silent_run) = summarize_command(
        &RepoCommandKind::RunMaintenance,
        &command_output("git maintenance run --auto", "", ""),
        true,
        None,
    );
    assert_eq!(silent_run, "Maintenance: Completed");

    let (_, skipped) = summarize_command(
        &RepoCommandKind::RunMaintenance,
        &command_output(gitcomet_core::services::MAINTENANCE_CHECK_COMMAND, "", ""),
        true,
        None,
    );
    assert_eq!(
        skipped,
        "Maintenance: Not needed, the repository is already optimized"
    );
}

#[test]
fn summarize_command_success_covers_status_variants() {
    let (_, fetch_summary) = summarize_command(
        &RepoCommandKind::FetchAll,
        &command_output("git fetch", "synced", ""),
        true,
        None,
    );
    assert_eq!(fetch_summary, "Fetch: Synchronized");

    let gitignore_command = RepoCommandKind::AppendGitignorePatterns {
        patterns: vec!["/build/out.log".to_string()],
    };
    let (_, gitignore_written) = summarize_command(
        &gitignore_command,
        &command_output("Update .gitignore", "", ""),
        true,
        None,
    );
    assert_eq!(gitignore_written, "Added /build/out.log to .gitignore");

    let (_, gitignore_noop) = summarize_command(
        &gitignore_command,
        &command_output(
            "Update .gitignore",
            gitcomet_core::gitignore::NOTHING_TO_ADD,
            "",
        ),
        true,
        None,
    );
    assert_eq!(
        gitignore_noop, "Already in .gitignore; nothing added",
        "the worker skipped the write, so announcing \"Added …\" would send \
             the user looking for a change that never happened"
    );

    let (_, gitignore_many) = summarize_command(
        &RepoCommandKind::AppendGitignorePatterns {
            patterns: vec!["/a".to_string(), "/b".to_string()],
        },
        &command_output("Update .gitignore", "", ""),
        true,
        None,
    );
    assert_eq!(gitignore_many, "Added 2 patterns to .gitignore");

    let (_, pull_up_to_date) = summarize_command(
        &RepoCommandKind::Pull {
            mode: PullMode::Default,
        },
        &command_output("git pull", "Already up to date", ""),
        true,
        None,
    );
    assert_eq!(pull_up_to_date, "Pull: Already up to date");

    let (_, pull_fast_forward) = summarize_command(
        &RepoCommandKind::Pull {
            mode: PullMode::Default,
        },
        &command_output("git pull", "Updating abc..def", ""),
        true,
        None,
    );
    assert_eq!(pull_fast_forward, "Pull: Fast-forwarded");

    let (_, pull_merged) = summarize_command(
        &RepoCommandKind::Pull {
            mode: PullMode::Default,
        },
        &command_output("git pull", "Merge branch 'feature'", ""),
        true,
        None,
    );
    assert_eq!(pull_merged, "Pull: Merged");

    let (_, pull_rebased) = summarize_command(
        &RepoCommandKind::Pull {
            mode: PullMode::Default,
        },
        &command_output(
            "git pull",
            "Successfully rebased and updated refs/heads/main.",
            "",
        ),
        true,
        None,
    );
    assert_eq!(pull_rebased, "Pull: Rebasing complete");

    let (_, pull_branch_summary) = summarize_command(
        &RepoCommandKind::PullBranch {
            remote: "origin".into(),
            branch: "main".into(),
        },
        &command_output("git pull origin main", "Updating abc..def", ""),
        true,
        None,
    );
    assert_eq!(pull_branch_summary, "Pull origin/main: Fast-forwarded");

    let (_, merge_ref_summary) = summarize_command(
        &RepoCommandKind::MergeRef {
            reference: "feature".into(),
        },
        &command_output("git merge feature", "Fast-forward", ""),
        true,
        None,
    );
    assert_eq!(merge_ref_summary, "Merge feature: Fast-forwarded");

    let (_, squash_ref_summary) = summarize_command(
        &RepoCommandKind::SquashRef {
            reference: "feature".into(),
        },
        &command_output(
            "git merge --squash feature",
            "Squash commit -- not updating HEAD\nAutomatic merge went well; stopped before committing as requested",
            "",
        ),
        true,
        None,
    );
    assert_eq!(squash_ref_summary, "Squash feature: Staged");

    let (_, push_uptodate) = summarize_command(
        &RepoCommandKind::Push,
        &command_output("git push", "", "Everything up-to-date"),
        true,
        None,
    );
    assert_eq!(push_uptodate, "Push: Everything up-to-date");

    let (_, force_push_uptodate) = summarize_command(
        &RepoCommandKind::ForcePush,
        &command_output("git push --force", "", "Everything up-to-date"),
        true,
        None,
    );
    assert_eq!(force_push_uptodate, "Force push: Everything up-to-date");

    let (_, push_upstream_uptodate) = summarize_command(
        &RepoCommandKind::PushSetUpstream {
            remote: "origin".into(),
            branch: "main".into(),
        },
        &command_output("git push -u origin main", "", "Everything up-to-date"),
        true,
        None,
    );
    assert_eq!(
        push_upstream_uptodate,
        "Push -u origin/main: Everything up-to-date"
    );

    let (_, set_upstream_summary) = summarize_command(
        &RepoCommandKind::SetUpstreamBranch {
            branch: "feature".into(),
            upstream: Upstream {
                remote: "origin".into(),
                branch: "feature".into(),
            },
        },
        &command_output(
            "git branch --set-upstream-to origin/feature feature",
            "",
            "",
        ),
        true,
        None,
    );
    assert_eq!(
        set_upstream_summary,
        "Branch feature: Upstream set to origin/feature"
    );

    let (_, unset_upstream_summary) = summarize_command(
        &RepoCommandKind::UnsetUpstreamBranch {
            branch: "feature".into(),
        },
        &command_output("git branch --unset-upstream feature", "", ""),
        true,
        None,
    );
    assert_eq!(unset_upstream_summary, "Branch feature: Upstream unlinked");

    let (_, push_tag_uptodate) = summarize_command(
        &RepoCommandKind::PushTag {
            remote: "origin".into(),
            name: "v1".into(),
        },
        &command_output("git push origin v1", "", "Everything up-to-date"),
        true,
        None,
    );
    assert_eq!(push_tag_uptodate, "Tag v1 → origin: Already up-to-date");

    let (_, reset_soft) = summarize_command(
        &RepoCommandKind::Reset {
            mode: ResetMode::Soft,
            target: "HEAD~1".into(),
        },
        &command_output("git reset --soft HEAD~1", "", ""),
        true,
        None,
    );
    assert_eq!(reset_soft, "Reset (--soft) HEAD~1: Completed");

    let (_, reset_mixed) = summarize_command(
        &RepoCommandKind::Reset {
            mode: ResetMode::Mixed,
            target: "HEAD~1".into(),
        },
        &command_output("git reset --mixed HEAD~1", "", ""),
        true,
        None,
    );
    assert_eq!(reset_mixed, "Reset (--mixed) HEAD~1: Completed");

    let (_, reset_hard) = summarize_command(
        &RepoCommandKind::Reset {
            mode: ResetMode::Hard,
            target: "HEAD~1".into(),
        },
        &command_output("git reset --hard HEAD~1", "", ""),
        true,
        None,
    );
    assert_eq!(reset_hard, "Reset (--hard) HEAD~1: Completed");

    let (_, rebase_summary) = summarize_command(
        &RepoCommandKind::Rebase {
            onto: "origin/main".into(),
        },
        &command_output("git rebase origin/main", "", ""),
        true,
        None,
    );
    assert_eq!(rebase_summary, "Rebase onto origin/main: Completed");

    let (_, rebase_continue_summary) = summarize_command(
        &RepoCommandKind::RebaseContinue,
        &command_output("git rebase --continue", "", ""),
        true,
        None,
    );
    assert_eq!(rebase_continue_summary, "Rebase: Continued");

    let (_, rebase_abort_summary) = summarize_command(
        &RepoCommandKind::RebaseAbort,
        &command_output("git rebase --abort", "", ""),
        true,
        None,
    );
    assert_eq!(rebase_abort_summary, "Rebase: Aborted");

    let (_, cherry_pick_continue_summary) = summarize_command(
        &RepoCommandKind::RebaseContinue,
        &command_output("git cherry-pick --continue", "", ""),
        true,
        None,
    );
    assert_eq!(cherry_pick_continue_summary, "Cherry-pick: Continued");

    let (_, cherry_pick_abort_summary) = summarize_command(
        &RepoCommandKind::RebaseAbort,
        &command_output("git cherry-pick --abort", "", ""),
        true,
        None,
    );
    assert_eq!(cherry_pick_abort_summary, "Cherry-pick: Aborted");

    for (command, kind, expected) in [
        (
            "git revert --continue",
            RepoCommandKind::RebaseContinue,
            "Revert: Continued",
        ),
        (
            gitcomet_core::services::REVERT_SKIP_COMMAND,
            RepoCommandKind::RebaseContinue,
            "Revert: Skipped the revert the resolution left empty",
        ),
        (
            "git revert --abort",
            RepoCommandKind::RebaseAbort,
            "Revert: Aborted",
        ),
    ] {
        let (_, summary) = summarize_command(&kind, &command_output(command, "", ""), true, None);
        assert_eq!(summary, expected, "{command}");
    }

    let (_, kept_head) = summarize_command(
        &RepoCommandKind::RebaseAbort,
        &command_output(
            "git revert --abort",
            gitcomet_core::services::REVERT_ABORT_KEPT_HEAD_SENTINEL,
            "",
        ),
        true,
        None,
    );
    assert_eq!(
        kept_head,
        "Revert: Sequence cleared; HEAD was left where it is"
    );

    let mut paused_cherry_pick = command_output("git cherry-pick --continue", "", "");
    paused_cherry_pick.exit_code = Some(1);
    let (_, cherry_pick_pause_summary) = summarize_command(
        &RepoCommandKind::RebaseContinue,
        &paused_cherry_pick,
        true,
        None,
    );
    assert_eq!(
        cherry_pick_pause_summary,
        "Cherry-pick: Paused at the next conflict"
    );

    let (_, interactive_rebase_summary) = summarize_command(
        &RepoCommandKind::InteractiveRebase {
            base: "HEAD~3".into(),
            interactive: true,
        },
        &command_output("git rebase -i HEAD~3", "", ""),
        true,
        None,
    );
    assert_eq!(
        interactive_rebase_summary,
        "Interactive rebase onto HEAD~3: Completed"
    );

    // An automated squash rebase (no editor window) reports as "Rebase".
    let (_, squash_rebase_summary) = summarize_command(
        &RepoCommandKind::InteractiveRebase {
            base: "HEAD~3".into(),
            interactive: false,
        },
        &command_output("git rebase -i HEAD~3", "", ""),
        true,
        None,
    );
    assert_eq!(squash_rebase_summary, "Rebase onto HEAD~3: Completed");

    let commit_id = CommitId("abcdef1234567890".into());
    let (_, cherry_pick_summary) = summarize_command(
        &RepoCommandKind::CherryPick {
            commit_id: commit_id.clone(),
            commit: true,
            mainline: None,
            summary: "fix parser\n\nbody".into(),
        },
        &command_output("git cherry-pick abcdef1", "", ""),
        true,
        None,
    );
    assert_eq!(cherry_pick_summary, "Cherry-picked abcdef1: fix parser");

    let (_, cherry_pick_no_commit_summary) = summarize_command(
        &RepoCommandKind::CherryPick {
            commit_id: commit_id.clone(),
            commit: false,
            mainline: None,
            summary: "fix parser".into(),
        },
        &command_output("git cherry-pick --no-commit abcdef1", "", ""),
        true,
        None,
    );
    assert_eq!(
        cherry_pick_no_commit_summary,
        "Cherry-picked abcdef1 without committing: fix parser"
    );

    let (_, cherry_pick_already_applied_summary) = summarize_command(
        &RepoCommandKind::CherryPick {
            commit_id,
            commit: true,
            mainline: None,
            summary: "fix parser".into(),
        },
        &command_output(
            "git cherry-pick abcdef1",
            "GITCOMET_CHERRY_PICK_ALREADY_APPLIED",
            "",
        ),
        true,
        None,
    );
    assert_eq!(
        cherry_pick_already_applied_summary,
        "Current branch already has all the changes from the cherry-picked commit."
    );

    let multi_pick = |commit: bool| RepoCommandKind::InteractiveCherryPick {
        entries: vec![
            gitcomet_core::services::InteractiveRebaseEntry {
                action: gitcomet_core::services::InteractiveRebaseAction::Pick,
                commit_id: "1111111".into(),
                summary: "one".into(),
                message: "one".into(),
                new_message: None,
            };
            2
        ],
        commit,
    };
    let multi_summary = |commit: bool, stdout: &str| {
        summarize_command(
            &multi_pick(commit),
            &command_output("git cherry-pick 2 commits", stdout, ""),
            true,
            None,
        )
        .1
    };
    assert_eq!(multi_summary(true, ""), "Cherry-pick 2 commits: Completed");
    assert_eq!(
        multi_summary(false, ""),
        "Cherry-picked 2 commits without committing"
    );
    assert_eq!(
        multi_summary(false, "GITCOMET_CHERRY_PICK_ALREADY_APPLIED"),
        "Current branch already has all the changes from the cherry-picked commits."
    );

    let apply_paths =
        |commit: bool, to: Option<&str>, paths: &[&str]| RepoCommandKind::ApplyFileChange {
            commit_retry: None,
            target: gitcomet_core::domain::ApplyChangeTarget {
                source: match to {
                    Some(to) => gitcomet_core::domain::ApplyChangeSource::Range {
                        from: CommitId("abcdef1234567890".into()),
                        to: CommitId(to.into()),
                    },
                    None => gitcomet_core::domain::ApplyChangeSource::Commit(CommitId(
                        "abcdef1234567890".into(),
                    )),
                },
                paths: paths.iter().map(PathBuf::from).collect(),
            },
            commit,
        };
    let apply = |commit: bool, to: Option<&str>| apply_paths(commit, to, &["src/a.rs"]);
    for (kind, stdout, expected) in [
        (
            apply(false, None),
            "",
            "Applied the change to src/a.rs from abcdef1 without committing",
        ),
        (
            apply(true, Some("1234567890abcdef")),
            "",
            "Applied and committed the change to src/a.rs from abcdef1..1234567",
        ),
        (
            apply(true, None),
            gitcomet_core::services::APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL,
            "Current branch already has the change to src/a.rs from abcdef1.",
        ),
        (
            apply_paths(false, None, &["src/a.rs", "src/b.rs"]),
            "",
            "Applied the changes to 2 files from abcdef1 without committing",
        ),
        (
            apply_paths(true, Some("1234567890abcdef"), &["a", "b", "c"]),
            "",
            "Applied and committed the changes to 3 files from abcdef1..1234567",
        ),
        (
            apply_paths(true, None, &["src/a.rs", "src/b.rs"]),
            gitcomet_core::services::APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL,
            "Current branch already has the changes to these 2 files from abcdef1.",
        ),
    ] {
        let (_, summary) = summarize_command(
            &kind,
            &command_output("git apply --3way src/a.rs", stdout, ""),
            true,
            None,
        );
        assert_eq!(summary, expected);
    }

    let revert = |commit: bool, summary: &str| RepoCommandKind::Revert {
        commit_id: CommitId("abcdef1234567890".into()),
        commit,
        mainline: None,
        summary: summary.into(),
    };
    for (kind, stdout, expected) in [
        (
            revert(true, "fix parser\n\nbody"),
            "",
            "Reverted abcdef1: fix parser",
        ),
        (
            revert(false, "fix parser"),
            "",
            "Reverted abcdef1 without committing: fix parser",
        ),
        (revert(true, ""), "", "Reverted abcdef1"),
        (
            revert(true, "fix parser"),
            gitcomet_core::services::REVERT_NOTHING_TO_REVERT_SENTINEL,
            "Nothing to revert: the current branch no longer has the changes from abcdef1.",
        ),
    ] {
        let (_, summary) = summarize_command(
            &kind,
            &command_output("git revert abcdef1", stdout, ""),
            true,
            None,
        );
        assert_eq!(summary, expected);
    }

    let (_, merge_abort_summary) = summarize_command(
        &RepoCommandKind::MergeAbort,
        &command_output("git merge --abort", "", ""),
        true,
        None,
    );
    assert_eq!(merge_abort_summary, "Merge: Aborted");

    let (_, create_tag_summary) = summarize_command(
        &RepoCommandKind::CreateTag {
            name: "v2".into(),
            target: "HEAD".into(),
            message: None,
            annotated: false,
        },
        &command_output("git tag v2 HEAD", "", ""),
        true,
        None,
    );
    assert_eq!(create_tag_summary, "Tag v2 → HEAD: Created");

    let (_, delete_tag_summary) = summarize_command(
        &RepoCommandKind::DeleteTag { name: "v2".into() },
        &command_output("git tag -d v2", "", ""),
        true,
        None,
    );
    assert_eq!(delete_tag_summary, "Tag v2: Deleted");

    let (_, add_remote_summary) = summarize_command(
        &RepoCommandKind::AddRemote {
            name: "origin".into(),
            url: "https://example.com/repo.git".into(),
        },
        &command_output("git remote add origin ...", "", ""),
        true,
        None,
    );
    assert_eq!(add_remote_summary, "Remote origin: Added");

    let (_, remove_remote_summary) = summarize_command(
        &RepoCommandKind::RemoveRemote {
            name: "origin".into(),
        },
        &command_output("git remote remove origin", "", ""),
        true,
        None,
    );
    assert_eq!(remove_remote_summary, "Remote origin: Removed");

    let (_, set_remote_url_summary) = summarize_command(
        &RepoCommandKind::SetRemoteUrl {
            name: "origin".into(),
            url: "https://example.com/repo.git".into(),
            kind: RemoteUrlKind::Push,
        },
        &command_output("git remote set-url --push origin ...", "", ""),
        true,
        None,
    );
    assert_eq!(set_remote_url_summary, "Remote origin (push): URL updated");
}

#[test]
fn error_format_helpers_preserve_backend_operation_and_path() {
    let error = Error::new(ErrorKind::Backend("reference target unavailable".into()))
        .with_context("resolve HEAD", Some(std::path::Path::new("repository")));

    assert_eq!(
        format_error_for_user(&error),
        "resolve HEAD (repository): reference target unavailable"
    );
    assert_eq!(
        format_failure_summary("Refresh", &error),
        "Refresh failed:\n\nresolve HEAD (repository): reference target unavailable"
    );
}

#[test]
fn timeout_summary_bounds_both_streams_without_losing_the_reason() {
    let command = "git -C /repo pull --rebase --no-prune -- origin refs/heads/main";
    let reason = "after 300 seconds without output or progress (set GITCOMET_GIT_COMMAND_TIMEOUT_SECS to override)";
    let stdout = format!(
        "discarded stdout prefix{}stdout tail\n",
        "x".repeat(300_000)
    );
    let stderr = format!(
        "discarded stderr prefix{}stderr tail\n",
        "y".repeat(300_000)
    );
    let error = Error::new(ErrorKind::Git(GitFailure::new(
        command,
        GitFailureId::Timeout,
        None,
        stdout.into_bytes(),
        stderr.into_bytes(),
        Some(reason.into()),
    )));
    let (actual_command, summary) = summarize_command(
        &RepoCommandKind::Pull {
            mode: PullMode::Rebase,
        },
        &CommandOutput::default(),
        false,
        Some(&error),
    );
    assert_eq!(actual_command, command);
    assert!(summary.starts_with("Pull timed out:"));
    assert!(summary.contains(reason));
    assert!(summary.contains("stdout tail"));
    assert!(summary.contains("stderr tail"));
    assert!(!summary.contains("discarded stdout prefix"));
    assert!(!summary.contains("discarded stderr prefix"));
    assert_eq!(summary.matches("[Earlier output was truncated]").count(), 2);
    assert!(summary.len() < 2 * 256 * 1024 + 1024);
}

#[test]
fn error_format_helpers_cover_non_git_and_failed_suffix_cases() {
    let git_error = Error::new(ErrorKind::Git(GitFailure::new(
        "git fetch --all",
        GitFailureId::CommandFailed,
        Some(128),
        Vec::new(),
        b"fatal: network down\n".to_vec(),
        Some("fatal: network down".to_string()),
    )));
    let formatted = format_failure_summary("Fetch", &git_error);
    assert!(formatted.contains("Fetch failed"));
    assert!(formatted.contains("git fetch --all"));
    assert!(formatted.contains("fatal: network down"));
    assert_eq!(
        format_error_for_user(&git_error),
        "git fetch --all failed: fatal: network down"
    );

    let backend_error = Error::new(ErrorKind::Backend(
        "git fetch --all failed: fatal: network down".to_string(),
    ));
    assert!(format_failure_summary("Fetch", &backend_error).contains("git fetch --all"));

    let io_error = Error::new(ErrorKind::Io(io::ErrorKind::Other));
    let io_rendered = format_error_for_user(&io_error);
    assert_eq!(io_rendered, io_error.to_string());
    assert!(!io_rendered.is_empty());
    assert!(try_format_git_backend_error(&io_error).is_none());
    assert!(try_format_git_backend_error_message("curl failed: timeout").is_none());
    assert_eq!(
        parse_failed_command_message("git status failed"),
        Some(("git status".to_string(), None))
    );

    let rendered = render_command_and_output("git status", Some(""));
    assert!(rendered.contains("    git status"));
    assert!(!rendered.contains("\n\n    "));

    assert_eq!(
        "value".to_string().if_empty_else(|| "fallback".to_string()),
        "value"
    );
}

#[test]
fn detect_auth_prompt_kind_classifies_username_password_passphrase_and_host_verification() {
    assert_eq!(
        detect_auth_prompt_kind_from_message(
            "git pull failed: fatal: could not read Username for 'https://example.com': terminal prompts disabled"
        ),
        Some(crate::model::AuthPromptKind::UsernamePassword)
    );
    assert_eq!(
        detect_auth_prompt_kind_from_message(
            "git push failed: Enter passphrase for key '/home/user/.ssh/id_ed25519': terminal prompts disabled"
        ),
        Some(crate::model::AuthPromptKind::Passphrase)
    );
    assert_eq!(
        detect_auth_prompt_kind_from_message(
            "git clone --progress git@github.com:org/repo.git C:\\git\\repo failed: git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository."
        ),
        None
    );
    assert_eq!(
        detect_auth_prompt_kind_from_message(
            "git pull --no-rebase origin main failed: Host key verification failed.\nfatal: Could not read from remote repository."
        ),
        None
    );
    assert_eq!(
        detect_auth_prompt_kind_from_message(
            "git fetch origin failed: The authenticity of host 'github.com (140.82.121.3)' can't be established.\nED25519 key fingerprint is: SHA256:+DiY...\nAre you sure you want to continue connecting (yes/no/[fingerprint])?"
        ),
        Some(crate::model::AuthPromptKind::HostVerification)
    );
    assert!(detect_auth_prompt_kind_from_message("git status failed").is_none());

    assert_eq!(
        detect_auth_prompt_kind_from_message(
            "git commit failed: error: Load key \"C:\\Users\\dev\\.ssh\\id_ed25519\": incorrect passphrase supplied to decrypt private key\nfatal: failed to write commit object"
        ),
        Some(crate::model::AuthPromptKind::Passphrase)
    );
    assert_eq!(
        detect_auth_prompt_kind_from_message(
            "git tag failed: Enter passphrase for \"/home/dev/.ssh/id_ed25519\":"
        ),
        Some(crate::model::AuthPromptKind::Passphrase)
    );
    assert_eq!(
        detect_auth_prompt_kind_from_message(&format!(
            "git commit failed\n{SSH_PASSPHRASE_PROMPT_MARKER}\nEnter passphrase for key"
        )),
        None
    );

    let structured = Error::new(ErrorKind::Git(GitFailure::new(
        "git fetch origin",
        GitFailureId::CommandFailed,
        Some(128),
        Vec::new(),
        b"Host key verification failed.\nfatal: Could not read from remote repository.\n".to_vec(),
        None,
    )));
    assert_eq!(detect_auth_prompt_kind(&structured), None);
}

#[test]
fn stage_git_auth_env_stages_and_clears_shared_auth_slot() {
    let _lock = crate::store::tests::staged_auth_test_lock();
    gitcomet_core::auth::clear_staged_git_auth();
    stage_git_auth_env(
        crate::model::AuthPromptKind::UsernamePassword,
        Some("alice"),
        "secret-token",
    )
    .expect("staging auth");

    let staged = gitcomet_core::auth::take_staged_git_auth().expect("staged auth to exist");
    assert_eq!(staged.username.as_deref(), Some("alice"));
    assert_eq!(staged.secret, "secret-token");
    assert_eq!(
        staged.kind,
        gitcomet_core::auth::GitAuthKind::UsernamePassword
    );

    stage_git_auth_env(
        crate::model::AuthPromptKind::Passphrase,
        None,
        "ssh-passphrase",
    )
    .expect("staging passphrase");

    let staged = gitcomet_core::auth::take_staged_git_auth().expect("staged passphrase to exist");
    assert_eq!(staged.kind, gitcomet_core::auth::GitAuthKind::Passphrase);

    stage_git_auth_env(
        crate::model::AuthPromptKind::HostVerification,
        None,
        " YES ",
    )
    .expect("staging host verification");

    let staged =
        gitcomet_core::auth::take_staged_git_auth().expect("staged host verification to exist");
    assert_eq!(
        staged.kind,
        gitcomet_core::auth::GitAuthKind::HostVerification
    );
    assert_eq!(staged.secret, "yes");

    clear_staged_git_auth_env();
    assert!(gitcomet_core::auth::take_staged_git_auth().is_none());
}
