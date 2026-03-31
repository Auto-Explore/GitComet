use gitcomet_core::domain::{LogScope, RepoSpec};
use gitcomet_state::model::{AppState, RepoId, RepoState};
use gitcomet_state::session::{self, UiSession, UiSettings};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SESSION_FILE_ENV: &str = "GITCOMET_SESSION_FILE";
const DISABLE_SESSION_PERSIST_ENV: &str = "GITCOMET_DISABLE_SESSION_PERSIST";

fn unique_temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gitcomet-state-session-integration-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_session_json(path: &Path, value: serde_json::Value) {
    let bytes = serde_json::to_vec(&value).expect("serialize session json");
    fs::write(path, bytes).expect("write session file");
}

fn env_defaults_available() -> bool {
    std::env::var_os(SESSION_FILE_ENV).is_none()
        && std::env::var_os(DISABLE_SESSION_PERSIST_ENV).is_none()
}

fn run_subtest_with_env(test_name: &str, session_file: &Path, disable_persist: bool) {
    let current_exe = std::env::current_exe().expect("locate current test binary");
    let mut cmd = Command::new(current_exe);
    cmd.arg("--exact").arg(test_name).arg("--nocapture");
    cmd.env(SESSION_FILE_ENV, session_file);
    if disable_persist {
        cmd.env(DISABLE_SESSION_PERSIST_ENV, "1");
    } else {
        cmd.env_remove(DISABLE_SESSION_PERSIST_ENV);
    }
    let output = cmd.output().expect("spawn subtest process");
    assert!(
        output.status.success(),
        "subtest {} failed:\nstdout:\n{}\nstderr:\n{}",
        test_name,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn wrapper_apis_return_defaults_without_session_path() {
    if !env_defaults_available() {
        eprintln!("skipping test because session env vars are preset");
        return;
    }

    assert_eq!(session::load(), UiSession::default());
    assert!(session::persist_from_state(&AppState::default()).is_ok());
    assert!(session::persist_ui_settings(UiSettings::default()).is_ok());

    let repo = Path::new("/tmp/gitcomet-session-wrapper");
    assert_eq!(session::load_repo_history_scope(repo), None);
    assert!(session::load_repo_history_scopes().is_empty());
    assert!(session::persist_repo_history_scope(repo, LogScope::CurrentBranch).is_ok());

    assert_eq!(
        session::load_repo_fetch_prune_deleted_remote_tracking_branches(repo),
        None
    );
    assert!(session::load_repo_fetch_prune_deleted_remote_tracking_branches_by_repo().is_empty());
    assert!(session::persist_repo_fetch_prune_deleted_remote_tracking_branches(repo, true).is_ok());
}

#[test]
fn wrapper_apis_use_session_file_env_when_set() {
    if std::env::var_os(SESSION_FILE_ENV).is_none() {
        return;
    }
    let repo = Path::new("/tmp/gitcomet-session-env-repo");

    let state = AppState {
        repos: vec![RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: repo.to_path_buf(),
            },
        )],
        active_repo: Some(RepoId(1)),
        ..Default::default()
    };

    session::persist_from_state(&state).expect("persist state through wrapper");
    let loaded = session::load();
    assert_eq!(loaded.open_repos, vec![repo.to_path_buf()]);
    assert_eq!(loaded.active_repo, Some(repo.to_path_buf()));

    session::persist_ui_settings(UiSettings {
        window_width: Some(900),
        window_height: Some(700),
        ..UiSettings::default()
    })
    .expect("persist ui settings through wrapper");
    let loaded = session::load();
    assert_eq!(loaded.window_width, Some(900));
    assert_eq!(loaded.window_height, Some(700));

    session::persist_repo_history_scope(repo, LogScope::CurrentBranch)
        .expect("persist history scope through wrapper");
    assert_eq!(
        session::load_repo_history_scope(repo),
        Some(LogScope::CurrentBranch)
    );
    let scopes = session::load_repo_history_scopes();
    assert_eq!(
        scopes.get(&repo.to_string_lossy().to_string()),
        Some(&LogScope::CurrentBranch)
    );

    session::persist_repo_fetch_prune_deleted_remote_tracking_branches(repo, false)
        .expect("persist fetch-prune through wrapper");
    assert_eq!(
        session::load_repo_fetch_prune_deleted_remote_tracking_branches(repo),
        Some(false)
    );
    let by_repo = session::load_repo_fetch_prune_deleted_remote_tracking_branches_by_repo();
    assert_eq!(
        by_repo.get(&repo.to_string_lossy().to_string()),
        Some(&false)
    );
}

#[test]
fn wrapper_apis_prefer_session_file_env_even_when_disable_flag_is_set() {
    if std::env::var_os(SESSION_FILE_ENV).is_some() {
        return;
    }

    let dir = unique_temp_dir("wrapper-env-subprocess");
    let session_file = dir.join("session.json");
    run_subtest_with_env(
        "wrapper_apis_use_session_file_env_when_set",
        &session_file,
        true,
    );
}

#[test]
fn load_from_path_handles_invalid_json_and_unknown_version() {
    let dir = unique_temp_dir("invalid-json");
    let session_file = dir.join("session.json");

    fs::write(&session_file, b"{not-json").expect("write malformed json");
    assert_eq!(session::load_from_path(&session_file), UiSession::default());

    write_session_json(
        &session_file,
        json!({
            "version": 999,
            "open_repos": ["/tmp/repo"],
            "active_repo": "/tmp/repo"
        }),
    );
    assert_eq!(session::load_from_path(&session_file), UiSession::default());
}

#[test]
fn load_from_path_parses_encoded_paths_and_filters_invalid_active_repo() {
    let dir = unique_temp_dir("path-keys");
    let session_file = dir.join("session.json");

    // Includes:
    // - duplicate plain path
    // - uppercase hex bytes (valid decode)
    // - odd-length hex (invalid decode, falls back to literal key)
    // - invalid-hex bytes (invalid decode, falls back to literal key)
    write_session_json(
        &session_file,
        json!({
            "version": 2,
            "open_repos": [
                "   ",
                "/tmp/alpha",
                "/tmp/alpha",
                "gitcomet-path-bytes:2F746D702F4A",
                "gitcomet-path-bytes:abc",
                "gitcomet-path-bytes:zz"
            ],
            "active_repo": "/tmp/not-listed"
        }),
    );

    let loaded = session::load_from_path(&session_file);
    let mut expected_open_repos = vec![PathBuf::from("/tmp/alpha")];
    #[cfg(unix)]
    expected_open_repos.push(PathBuf::from("/tmp/J"));
    #[cfg(not(unix))]
    expected_open_repos.push(PathBuf::from("gitcomet-path-bytes:2F746D702F4A"));
    expected_open_repos.push(PathBuf::from("gitcomet-path-bytes:abc"));
    expected_open_repos.push(PathBuf::from("gitcomet-path-bytes:zz"));
    assert_eq!(loaded.open_repos, expected_open_repos);
    assert_eq!(loaded.active_repo, None);

    write_session_json(
        &session_file,
        json!({
            "version": 2,
            "open_repos": ["/tmp/alpha"],
            "active_repo": "   "
        }),
    );
    let loaded = session::load_from_path(&session_file);
    assert_eq!(loaded.active_repo, None);
}

#[test]
fn persist_from_state_to_path_deduplicates_repos_and_filters_missing_active_id() {
    let dir = unique_temp_dir("persist-state");
    let session_file = dir.join("session.json");
    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");
    fs::create_dir_all(&repo_a).expect("create repo_a");
    fs::create_dir_all(&repo_b).expect("create repo_b");

    let state = AppState {
        repos: vec![
            RepoState::new_opening(
                RepoId(1),
                RepoSpec {
                    workdir: repo_a.clone(),
                },
            ),
            RepoState::new_opening(
                RepoId(2),
                RepoSpec {
                    workdir: repo_a.clone(),
                },
            ),
            RepoState::new_opening(
                RepoId(3),
                RepoSpec {
                    workdir: repo_b.clone(),
                },
            ),
        ],
        active_repo: Some(RepoId(2)),
        ..Default::default()
    };

    session::persist_from_state_to_path(&state, &session_file).expect("persist deduped state");
    let loaded = session::load_from_path(&session_file);
    assert_eq!(loaded.open_repos, vec![repo_a.clone(), repo_b.clone()]);
    assert_eq!(loaded.active_repo, Some(repo_a));

    let state = AppState {
        active_repo: Some(RepoId(999)),
        ..state
    };
    session::persist_from_state_to_path(&state, &session_file)
        .expect("persist with missing active id");
    let loaded = session::load_from_path(&session_file);
    assert_eq!(loaded.active_repo, None);
}

#[test]
fn persist_ui_settings_to_path_updates_optional_fields_and_requires_both_window_dims() {
    let dir = unique_temp_dir("ui-settings");
    let session_file = dir.join("session.json");

    write_session_json(
        &session_file,
        json!({
            "version": 2,
            "open_repos": [],
            "active_repo": null,
            "window_width": 100,
            "window_height": 200
        }),
    );

    session::persist_ui_settings_to_path(
        UiSettings {
            window_width: Some(300),
            window_height: None,
            sidebar_width: Some(42),
            details_width: Some(84),
            repo_sidebar_collapsed_items: None,
            theme_mode: Some("light".to_string()),
            ui_font_family: Some(".SystemUIFont".to_string()),
            editor_font_family: Some("JetBrains Mono".to_string()),
            use_font_ligatures: Some(false),
            date_time_format: Some("ymd_hm_utc".to_string()),
            timezone: Some("UTC".to_string()),
            show_timezone: Some(true),
            change_tracking_view: Some("split_untracked".to_string()),
            change_tracking_height: Some(222),
            untracked_height: Some(111),
            history_show_author: Some(false),
            history_show_date: Some(true),
            history_show_sha: Some(false),
        },
        &session_file,
    )
    .expect("persist partial settings");

    let loaded = session::load_from_path(&session_file);
    assert_eq!(loaded.window_width, Some(100));
    assert_eq!(loaded.window_height, Some(200));
    assert_eq!(loaded.sidebar_width, Some(42));
    assert_eq!(loaded.details_width, Some(84));
    assert_eq!(loaded.theme_mode.as_deref(), Some("light"));
    assert_eq!(loaded.ui_font_family.as_deref(), Some(".SystemUIFont"));
    assert_eq!(loaded.editor_font_family.as_deref(), Some("JetBrains Mono"));
    assert_eq!(loaded.use_font_ligatures, Some(false));
    assert_eq!(loaded.date_time_format.as_deref(), Some("ymd_hm_utc"));
    assert_eq!(loaded.timezone.as_deref(), Some("UTC"));
    assert_eq!(loaded.show_timezone, Some(true));
    assert_eq!(
        loaded.change_tracking_view.as_deref(),
        Some("split_untracked")
    );
    assert_eq!(loaded.change_tracking_height, Some(222));
    assert_eq!(loaded.untracked_height, Some(111));
    assert_eq!(loaded.history_show_author, Some(false));
    assert_eq!(loaded.history_show_date, Some(true));
    assert_eq!(loaded.history_show_sha, Some(false));

    session::persist_ui_settings_to_path(
        UiSettings {
            window_width: Some(640),
            window_height: Some(480),
            ..UiSettings::default()
        },
        &session_file,
    )
    .expect("persist complete dimensions");

    let loaded = session::load_from_path(&session_file);
    assert_eq!(loaded.window_width, Some(640));
    assert_eq!(loaded.window_height, Some(480));
}

#[test]
fn sidebar_collapse_state_round_trips_via_ui_settings() {
    let dir = unique_temp_dir("sidebar-collapse");
    let session_file = dir.join("session.json");
    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");

    let mut repo_sidebar_collapsed_items = BTreeMap::new();
    repo_sidebar_collapsed_items.insert(
        repo_a.clone(),
        BTreeSet::from([
            "section:branches".to_string(),
            "section:worktrees".to_string(),
            "group:local:feature".to_string(),
        ]),
    );
    repo_sidebar_collapsed_items.insert(
        repo_b.clone(),
        BTreeSet::from(["group:remote:origin:release".to_string()]),
    );

    session::persist_ui_settings_to_path(
        UiSettings {
            repo_sidebar_collapsed_items: Some(repo_sidebar_collapsed_items.clone()),
            ..UiSettings::default()
        },
        &session_file,
    )
    .expect("persist sidebar collapse state");

    let loaded = session::load_from_path(&session_file);
    assert_eq!(
        loaded.repo_sidebar_collapsed_items,
        repo_sidebar_collapsed_items
    );
}

#[test]
fn history_scope_round_trips_for_individual_and_bulk_loaders() {
    let dir = unique_temp_dir("history-scope");
    let session_file = dir.join("session.json");
    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");

    session::persist_repo_history_scope_to_path(&repo_a, LogScope::CurrentBranch, &session_file)
        .expect("persist current-branch scope");
    session::persist_repo_history_scope_to_path(&repo_b, LogScope::AllBranches, &session_file)
        .expect("persist all-branches scope");

    assert_eq!(
        session::load_repo_history_scope_from_path(&repo_a, &session_file),
        Some(LogScope::CurrentBranch)
    );
    assert_eq!(
        session::load_repo_history_scope_from_path(&repo_b, &session_file),
        Some(LogScope::AllBranches)
    );
    assert_eq!(
        session::load_repo_history_scope_from_path(Path::new("/tmp/missing"), &session_file),
        None
    );

    let scopes = session::load_repo_history_scopes_from_path(&session_file);
    assert_eq!(scopes.len(), 2);
    assert_eq!(
        scopes.get(&repo_a.to_string_lossy().to_string()),
        Some(&LogScope::CurrentBranch)
    );
    assert_eq!(
        scopes.get(&repo_b.to_string_lossy().to_string()),
        Some(&LogScope::AllBranches)
    );

    let missing = session::load_repo_history_scopes_from_path(&dir.join("does-not-exist.json"));
    assert!(missing.is_empty());
}

#[test]
fn fetch_prune_round_trips_for_individual_and_bulk_loaders() {
    let dir = unique_temp_dir("fetch-prune");
    let session_file = dir.join("session.json");
    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");

    session::persist_repo_fetch_prune_deleted_remote_tracking_branches_to_path(
        &repo_a,
        true,
        &session_file,
    )
    .expect("persist repo_a setting");
    session::persist_repo_fetch_prune_deleted_remote_tracking_branches_to_path(
        &repo_b,
        false,
        &session_file,
    )
    .expect("persist repo_b setting");

    assert_eq!(
        session::load_repo_fetch_prune_deleted_remote_tracking_branches_from_path(
            &repo_a,
            &session_file,
        ),
        Some(true)
    );
    assert_eq!(
        session::load_repo_fetch_prune_deleted_remote_tracking_branches_from_path(
            &repo_b,
            &session_file,
        ),
        Some(false)
    );
    assert_eq!(
        session::load_repo_fetch_prune_deleted_remote_tracking_branches_from_path(
            Path::new("/tmp/missing"),
            &session_file,
        ),
        None
    );

    let by_repo = session::load_repo_fetch_prune_deleted_remote_tracking_branches_by_repo_from_path(
        &session_file,
    );
    assert_eq!(by_repo.len(), 2);
    assert_eq!(
        by_repo.get(&repo_a.to_string_lossy().to_string()),
        Some(&true)
    );
    assert_eq!(
        by_repo.get(&repo_b.to_string_lossy().to_string()),
        Some(&false)
    );

    let missing = session::load_repo_fetch_prune_deleted_remote_tracking_branches_by_repo_from_path(
        &dir.join("does-not-exist.json"),
    );
    assert!(missing.is_empty());
}
