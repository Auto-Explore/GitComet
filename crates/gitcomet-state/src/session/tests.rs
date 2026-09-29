use super::history_mode::HistoryScopeSetting;
#[cfg(unix)]
use super::paths::hex_encode;
use super::repos::SESSION_REPOS_SNAPSHOT_CACHE;
use super::survey::SurveyPromptSession;
use super::*;
use crate::model::{RepoId, RepoState};
use gitcomet_core::domain::{HistoryMode, LogScope, RepoSpec};

fn clear_session_repos_snapshot_cache() {
    SESSION_REPOS_SNAPSHOT_CACHE.with(|cache| {
        cache.borrow_mut().take();
    });
}

fn unique_session_test_dir(label: &str) -> PathBuf {
    let dir = env::temp_dir().join(format!(
        "gitcomet-session-unit-test-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    dir
}

#[test]
fn signature_verification_requires_fresh_opt_in_and_survives_other_writers() {
    let dir = unique_session_test_dir("signature-opt-in");
    let path = dir.join("session.json");
    for legacy in ["true", "false", "null"] {
        fs::write(&path, format!(r#"{{"version":3,"open_repos":[],"active_repo":null,"history_verify_commit_signatures":{legacy}}}"#)).unwrap();
        assert_eq!(
            load_from_path(&path).history_verify_commit_signatures,
            Some(false)
        );
        persist_ui_settings_to_path(
            UiSettings {
                window_width: Some(1200),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        assert_eq!(
            load_from_path(&path).history_verify_commit_signatures,
            Some(false)
        );
        persist_ui_settings_to_path(
            UiSettings {
                history_verify_commit_signatures: Some(true),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        persist_ui_settings_to_path(
            UiSettings {
                window_height: Some(800),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        assert_eq!(
            load_from_path(&path).history_verify_commit_signatures,
            Some(true)
        );
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["history_verify_commit_signatures_opt_in"], true);
        assert_eq!(saved["history_verify_commit_signatures"], true);
        assert_eq!(saved["version"], CURRENT_SESSION_FILE_VERSION);
    }
}

#[test]
fn history_branch_names_survives_other_session_writers() {
    let dir = unique_session_test_dir("history-branch-names");
    let path = dir.join("session.json");
    assert_eq!(load_from_path(&path).history_branch_names, None);
    for mode in ["inline", "separate_column"] {
        persist_ui_settings_to_path(
            UiSettings {
                history_branch_names: Some(mode.into()),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        // A later geometry/settings snapshot must not reset placement.
        persist_ui_settings_to_path(
            UiSettings {
                history_show_graph: Some(false),
                window_width: Some(1200),
                window_height: Some(800),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        let loaded = load_from_path(&path);
        assert_eq!(loaded.history_branch_names.as_deref(), Some(mode));
        assert_eq!(loaded.history_show_graph, Some(false));
        assert_eq!(loaded.window_width, Some(1200));
    }
    fs::remove_dir_all(dir).unwrap();
}

fn assert_session_writer_waits_for_shared_lock(
    label: &str,
    persist: impl FnOnce(PathBuf) -> io::Result<()> + Send + 'static,
) {
    let path = unique_session_test_dir(label).join("session.json");
    let guard = session_file_persist_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();

    let handle = std::thread::spawn(move || {
        started_tx.send(()).expect("send writer started");
        let result = persist(path);
        done_tx.send(result).expect("send writer result");
    });

    started_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("writer thread started");
    assert!(
        done_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err(),
        "{label} writer finished while the session persist lock was held"
    );
    drop(guard);

    done_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("writer finished after lock release")
        .expect("writer persist succeeds");
    handle.join().expect("writer thread joins");
}

#[test]
fn session_file_persist_lock_is_shared_by_session_writers() {
    assert_session_writer_waits_for_shared_lock("persist-repos-snapshot", |path| {
        let repo = path.with_file_name("repo-snapshot");
        let repo_text = repo.to_string_lossy().into_owned();
        let open_repos: Arc<[Arc<str>]> =
            Arc::from(vec![Arc::<str>::from(repo_text)].into_boxed_slice());
        let snapshot = SessionReposSnapshot {
            open_repos,
            active_repo_index: Some(0),
        };
        persist_repos_snapshot_to_path(&snapshot, &path)
    });
    assert_session_writer_waits_for_shared_lock("persist-recent-repo", |path| {
        let repo = path.with_file_name("recent-repo");
        fs::create_dir_all(&repo)?;
        persist_recent_repo_to_path(&repo, &path)
    });
    assert_session_writer_waits_for_shared_lock("remove-recent-repo", |path| {
        let repo = path.with_file_name("remove-recent-repo");
        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                recent_repos: Some(vec![path_storage_key(&repo)]),
                ..UiSessionFile::default()
            },
        )?;
        remove_recent_repo_to_path(&repo, &path)
    });
    assert_session_writer_waits_for_shared_lock("persist-ui-settings", |path| {
        persist_ui_settings_to_path(
            UiSettings {
                external_code_editor: Some(Some(ExternalCodeEditorSetting::Custom {
                    executable: PathBuf::from("/usr/bin/editor"),
                    arguments: Some("--reuse-window".to_string()),
                })),
                ..UiSettings::default()
            },
            &path,
        )
    });
    assert_session_writer_waits_for_shared_lock("persist-history-mode", |path| {
        let repo = path.with_file_name("history-mode-repo");
        persist_repo_history_mode_to_path(&repo, HistoryMode::NoMerges, &path)
    });
    assert_session_writer_waits_for_shared_lock("persist-history-mode-batch", |path| {
        let repo = path.with_file_name("history-mode-batch-repo");
        persist_repo_history_modes_batch_to_path(&[(repo, HistoryMode::FirstParent)], &path)
    });
    assert_session_writer_waits_for_shared_lock("persist-history-scope", |path| {
        let repo = path.with_file_name("history-scope-repo");
        persist_repo_history_scope_to_path(&repo, LogScope::AllBranches, &path)
    });
    assert_session_writer_waits_for_shared_lock("persist-survey-opened", |path| {
        persist_survey_prompt_opened_to_path(&path, "survey", 123)
    });
    assert_session_writer_waits_for_shared_lock("persist-survey-postponed", |path| {
        persist_survey_prompt_postponed_to_path(&path, "survey", 60, 123)
    });
}

#[test]
fn session_file_round_trips() {
    let dir = env::temp_dir().join(format!("gitcomet-session-test-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    let file = UiSessionFileV1 {
        version: SESSION_FILE_VERSION_V1,
        open_repos: vec!["/a".into(), "/b".into()],
        active_repo: Some("/b".into()),
    };
    persist_to_path(&path, &file).expect("persist succeeds");

    let contents = fs::read_to_string(&path).expect("read succeeds");
    let loaded: UiSessionFileV1 = serde_json::from_str(&contents).expect("json parses");
    assert_eq!(loaded.version, SESSION_FILE_VERSION_V1);
    assert_eq!(loaded.open_repos, vec!["/a".to_string(), "/b".to_string()]);
    assert_eq!(loaded.active_repo.as_deref(), Some("/b"));
}

#[test]
fn path_storage_key_keeps_utf8_plain_text() {
    let path = Path::new("/tmp/gitcomet-repo");
    let key = path_storage_key(path);
    assert_eq!(key, "/tmp/gitcomet-repo");
    assert_eq!(path_from_storage_key(&key), path);
}

#[cfg(unix)]
#[test]
fn path_storage_key_round_trips_non_utf8_unix_bytes() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let path = Path::new(OsStr::from_bytes(b"/tmp/gitcomet-\xff"));
    let key = path_storage_key(path);
    assert!(key.starts_with(SESSION_PATH_BYTES_PREFIX), "{key}");
    let restored = path_from_storage_key(&key);
    assert_eq!(restored.as_os_str().as_bytes(), path.as_os_str().as_bytes());
}

#[test]
fn load_repo_session_preferences_collects_current_and_legacy_history_settings() {
    let dir = unique_session_test_dir("repo-session-preferences");
    let session_file = dir.join("session.json");
    let repo_mode = dir.join("repo-mode");
    let repo_legacy = dir.join("repo-legacy");
    let _ = fs::create_dir_all(&repo_mode);
    let _ = fs::create_dir_all(&repo_legacy);

    assert_eq!(
        load_repo_session_preferences_from_path(&dir.join("missing.json")),
        RepoSessionPreferences::default()
    );

    persist_ui_settings_to_path(
        UiSettings {
            default_history_mode: Some(HistoryMode::MergesOnly),
            ..UiSettings::default()
        },
        &session_file,
    )
    .expect("persist default history mode");
    persist_repo_history_mode_to_path(&repo_mode, HistoryMode::NoMerges, &session_file)
        .expect("persist explicit history mode");
    persist_repo_history_scope_to_path(&repo_legacy, LogScope::CurrentBranch, &session_file)
        .expect("persist legacy history scope");

    let loaded = load_repo_session_preferences_from_path(&session_file);
    assert_eq!(loaded.default_history_mode, Some(HistoryMode::MergesOnly));
    assert_eq!(
        loaded.repo_history_modes.get(&path_storage_key(&repo_mode)),
        Some(&HistoryMode::NoMerges)
    );
    assert_eq!(
        loaded
            .repo_history_scopes
            .get(&path_storage_key(&repo_legacy)),
        Some(&HistoryMode::FirstParent)
    );
}

#[test]
fn persist_ui_settings_round_trips_sidebar_collapsed() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-session-sidebar-collapsed-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let session_file = dir.join("session.json");

    // Default (unset) leaves the field absent.
    assert_eq!(load_from_path(&session_file).sidebar_collapsed, None);

    persist_ui_settings_to_path(
        UiSettings {
            sidebar_collapsed: Some(true),
            ..UiSettings::default()
        },
        &session_file,
    )
    .expect("persist collapsed");
    assert_eq!(load_from_path(&session_file).sidebar_collapsed, Some(true));

    // A later settings write that doesn't touch the field preserves it.
    persist_ui_settings_to_path(
        UiSettings {
            theme_mode: Some("dark".to_string()),
            ..UiSettings::default()
        },
        &session_file,
    )
    .expect("persist theme");
    assert_eq!(load_from_path(&session_file).sidebar_collapsed, Some(true));

    persist_ui_settings_to_path(
        UiSettings {
            sidebar_collapsed: Some(false),
            ..UiSettings::default()
        },
        &session_file,
    )
    .expect("persist expanded");
    assert_eq!(load_from_path(&session_file).sidebar_collapsed, Some(false));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn persist_repo_history_modes_batch_skips_empty_and_unchanged_updates() {
    let dir = unique_session_test_dir("repo-history-mode-batch");
    let session_file = dir.join("session.json");
    let missing_file = dir.join("missing.json");
    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");
    let repo_c = dir.join("repo-c");
    let _ = fs::create_dir_all(&repo_a);
    let _ = fs::create_dir_all(&repo_b);
    let _ = fs::create_dir_all(&repo_c);

    persist_repo_history_modes_batch_to_path(&[], &missing_file)
        .expect("empty updates should succeed");
    assert!(
        !missing_file.exists(),
        "empty batch updates should not create a session file"
    );

    persist_ui_settings_to_path(
        UiSettings {
            default_history_mode: Some(HistoryMode::MergesOnly),
            ..UiSettings::default()
        },
        &session_file,
    )
    .expect("persist default history mode");
    persist_repo_history_scope_to_path(&repo_b, LogScope::CurrentBranch, &session_file)
        .expect("persist legacy history scope");
    persist_repo_history_mode_to_path(&repo_a, HistoryMode::FirstParent, &session_file)
        .expect("persist repo_a history mode");

    let before = fs::read_to_string(&session_file).expect("read session file");

    persist_repo_history_modes_batch_to_path(&[], &session_file)
        .expect("empty updates should not rewrite the file");
    assert_eq!(
        fs::read_to_string(&session_file).expect("read session file after empty batch"),
        before
    );

    persist_repo_history_modes_batch_to_path(
        &[(repo_a.clone(), HistoryMode::FirstParent)],
        &session_file,
    )
    .expect("unchanged updates should not rewrite the file");
    assert_eq!(
        fs::read_to_string(&session_file).expect("read session file after unchanged batch"),
        before
    );

    persist_repo_history_modes_batch_to_path(
        &[
            (repo_b.clone(), HistoryMode::AllBranches),
            (repo_c.clone(), HistoryMode::NoMerges),
        ],
        &session_file,
    )
    .expect("persist changed batch updates");

    let loaded = load_repo_session_preferences_from_path(&session_file);
    assert_eq!(loaded.default_history_mode, Some(HistoryMode::MergesOnly));
    assert_eq!(
        loaded.repo_history_modes.get(&path_storage_key(&repo_a)),
        Some(&HistoryMode::FirstParent)
    );
    assert_eq!(
        loaded.repo_history_modes.get(&path_storage_key(&repo_b)),
        Some(&HistoryMode::AllBranches)
    );
    assert_eq!(
        loaded.repo_history_modes.get(&path_storage_key(&repo_c)),
        Some(&HistoryMode::NoMerges)
    );
    assert_eq!(
        loaded.repo_history_scopes.get(&path_storage_key(&repo_b)),
        Some(&HistoryMode::FirstParent)
    );
}

#[test]
fn survey_prompt_requires_recorded_repository() {
    const SURVEY_ID: &str = "gitcomet_user_survey_2026_04";
    let dir = unique_session_test_dir("survey-empty-session");
    let session_file = dir.join("session.json");

    assert!(!should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100
    ));

    fs::write(&session_file, b"{not-json").expect("write malformed session");
    assert!(!should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100
    ));

    fs::write(&session_file, br#"{"version":3}"#).expect("write version-only session");
    assert!(!should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100
    ));

    persist_to_path(
        &session_file,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            survey_prompt: Some(SurveyPromptSession {
                survey_id: SURVEY_ID.to_string(),
                opened_at_unix_seconds: None,
                postponed_until_unix_seconds: Some(50),
            }),
            ..UiSessionFile::default()
        },
    )
    .expect("persist survey-only session");
    assert!(!should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100
    ));

    persist_to_path(
        &session_file,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            window_width: Some(1200),
            window_height: Some(800),
            theme_mode: Some("dark".to_string()),
            repo_history_scopes: Some(BTreeMap::from([(
                "/tmp/repo".to_string(),
                HistoryScopeSetting::AllBranches,
            )])),
            ..UiSessionFile::default()
        },
    )
    .expect("persist non-repo session data");
    assert!(!should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100
    ));
}

#[test]
fn survey_prompt_accepts_recorded_repository_sources() {
    const SURVEY_ID: &str = "gitcomet_user_survey_2026_04";
    let dir = unique_session_test_dir("survey-repository-sources");
    let session_file = dir.join("session.json");

    persist_to_path(
        &session_file,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: vec![" /tmp/open-repo ".to_string()],
            ..UiSessionFile::default()
        },
    )
    .expect("persist open repo session");
    assert!(should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100
    ));

    persist_to_path(
        &session_file,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            active_repo: Some("/tmp/active-repo".to_string()),
            ..UiSessionFile::default()
        },
    )
    .expect("persist active repo session");
    assert!(should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100
    ));

    persist_to_path(
        &session_file,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            recent_repos: Some(vec!["\t/tmp/recent-repo\n".to_string()]),
            ..UiSessionFile::default()
        },
    )
    .expect("persist recent repo session");
    assert!(should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100
    ));
}

#[test]
fn survey_prompt_respects_id_opened_and_postponed_state() {
    const SURVEY_ID: &str = "gitcomet_user_survey_2026_04";
    const NEXT_SURVEY_ID: &str = "gitcomet_user_survey_2026_05";
    const POSTPONE_SECONDS: u64 = 60 * 60 * 24 * 7;
    let dir = unique_session_test_dir("survey-prompt-state");
    let session_file = dir.join("session.json");

    persist_to_path(
        &session_file,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: vec!["/tmp/repo".to_string()],
            ..UiSessionFile::default()
        },
    )
    .expect("persist eligible session");
    assert!(should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100
    ));

    persist_survey_prompt_postponed_to_path(&session_file, SURVEY_ID, POSTPONE_SECONDS, 100)
        .expect("persist postponed survey");
    let postponed_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&session_file).expect("read postponed session"))
            .expect("postponed session json parses");
    assert_eq!(
        postponed_json
            .pointer("/survey_prompt/survey_id")
            .and_then(|value| value.as_str()),
        Some(SURVEY_ID)
    );
    assert_eq!(
        postponed_json
            .pointer("/survey_prompt/postponed_until_unix_seconds")
            .and_then(|value| value.as_u64()),
        Some(100 + POSTPONE_SECONDS)
    );
    assert!(
        postponed_json
            .pointer("/survey_prompt/opened_at_unix_seconds")
            .is_none()
    );
    assert!(!should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100 + POSTPONE_SECONDS - 1
    ));
    assert!(should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        100 + POSTPONE_SECONDS
    ));
    assert!(should_show_survey_prompt_from_path(
        &session_file,
        NEXT_SURVEY_ID,
        100
    ));

    persist_survey_prompt_opened_to_path(&session_file, SURVEY_ID, 200)
        .expect("persist opened survey");
    let opened_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&session_file).expect("read opened session"))
            .expect("opened session json parses");
    assert_eq!(
        opened_json
            .pointer("/survey_prompt/survey_id")
            .and_then(|value| value.as_str()),
        Some(SURVEY_ID)
    );
    assert_eq!(
        opened_json
            .pointer("/survey_prompt/opened_at_unix_seconds")
            .and_then(|value| value.as_u64()),
        Some(200)
    );
    assert!(
        opened_json
            .pointer("/survey_prompt/postponed_until_unix_seconds")
            .is_none()
    );
    assert!(!should_show_survey_prompt_from_path(
        &session_file,
        SURVEY_ID,
        300
    ));
    assert!(should_show_survey_prompt_from_path(
        &session_file,
        NEXT_SURVEY_ID,
        300
    ));
}

#[test]
fn survey_prompt_persistence_preserves_existing_session_fields() {
    const SURVEY_ID: &str = "gitcomet_user_survey_2026_04";
    let dir = unique_session_test_dir("survey-preserves-session");
    let session_file = dir.join("session.json");
    let repo = dir.join("repo");

    persist_to_path(
        &session_file,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: vec![path_storage_key(&repo)],
            active_repo: Some(path_storage_key(&repo)),
            recent_repos: Some(vec![path_storage_key(&repo)]),
            theme_mode: Some("dark".to_string()),
            repo_history_scopes: Some(BTreeMap::from([(
                path_storage_key(&repo),
                HistoryScopeSetting::AllBranches,
            )])),
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_survey_prompt_opened_to_path(&session_file, SURVEY_ID, 123)
        .expect("persist survey opened");

    let file = load_file(&session_file).expect("load session file");
    assert_eq!(file.open_repos, vec![path_storage_key(&repo)]);
    assert_eq!(
        file.active_repo.as_deref(),
        Some(path_storage_key(&repo).as_str())
    );
    assert_eq!(file.theme_mode.as_deref(), Some("dark"));
    assert_eq!(
        file.repo_history_scopes
            .as_ref()
            .and_then(|scopes| scopes.get(&path_storage_key(&repo))),
        Some(&HistoryScopeSetting::AllBranches)
    );
    assert_eq!(
        file.survey_prompt,
        Some(SurveyPromptSession {
            survey_id: SURVEY_ID.to_string(),
            opened_at_unix_seconds: Some(123),
            postponed_until_unix_seconds: None,
        })
    );
}

#[test]
fn detects_test_harness_executable_paths() {
    // `cargo test` / nextest binaries are typically located under a `deps` directory.
    assert!(looks_like_test_binary(Path::new(
        "/tmp/target/debug/deps/foo"
    )));
    assert!(!looks_like_test_binary(Path::new("/tmp/target/debug/foo")));

    // nextest uses a separate target subdir.
    assert!(looks_like_test_binary(Path::new(
        "/tmp/target/nextest/default/foo"
    )));

    // Cargo test binaries also have a hash suffix.
    assert!(looks_like_test_binary(Path::new(
        "/tmp/target/debug/gitcomet_ui_gpui-3ad1b0fd3f0c0d3e"
    )));
    assert!(!looks_like_test_binary(Path::new(
        "/tmp/target/debug/gitcomet"
    )));
}

#[cfg(target_os = "linux")]
#[test]
fn app_data_dir_prefers_xdg_data_home() {
    assert_eq!(
        app_data_dir_linux(
            Some(OsStr::new("/tmp/gitcomet-data")),
            Some(OsStr::new("/home/alice"))
        ),
        Some(PathBuf::from("/tmp/gitcomet-data/gitcomet"))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn app_data_dir_falls_back_to_local_share() {
    assert_eq!(
        app_data_dir_linux(None, Some(OsStr::new("/home/alice"))),
        Some(PathBuf::from("/home/alice/.local/share/gitcomet"))
    );
}

#[test]
fn persist_from_state_and_load_from_path_round_trip() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-session-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");
    let _ = fs::create_dir_all(&repo_a);
    let _ = fs::create_dir_all(&repo_b);

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
                    workdir: repo_b.clone(),
                },
            ),
        ],
        active_repo: Some(RepoId(2)),
        ..AppState::test_default()
    };

    persist_from_state_to_path(&state, &path).expect("persist succeeds");
    let loaded = load_from_path(&path);
    assert_eq!(loaded.open_repos, vec![repo_a, repo_b.clone()]);
    assert_eq!(loaded.active_repo, Some(repo_b));
}

#[test]
fn snapshot_repos_from_state_dedups_and_filters_inactive_selection() {
    let repo_a = PathBuf::from("/tmp/repo-a");
    let repo_b = PathBuf::from("/tmp/repo-b");
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
        ],
        active_repo: Some(RepoId(999)),
        ..AppState::test_default()
    };

    let snapshot = snapshot_repos_from_state(&state);
    assert_eq!(
        snapshot.open_repos.as_ref(),
        &[path_storage_key_shared(&repo_a)]
    );
    assert_eq!(snapshot.active_repo_index, None);

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
                    workdir: repo_b.clone(),
                },
            ),
        ],
        active_repo: Some(RepoId(2)),
        ..AppState::test_default()
    };
    let snapshot = snapshot_repos_from_state(&state);
    assert_eq!(snapshot.active_repo_index, Some(1));
    assert_eq!(snapshot.open_repos[1].as_ref(), "/tmp/repo-b");
}

#[test]
fn snapshot_repos_from_state_reuses_cached_open_repo_slice_for_same_repo_list() {
    let state = AppState {
        repos: vec![
            RepoState::new_opening(
                RepoId(1),
                RepoSpec {
                    workdir: PathBuf::from("/tmp/repo-a"),
                },
            ),
            RepoState::new_opening(
                RepoId(2),
                RepoSpec {
                    workdir: PathBuf::from("/tmp/repo-b"),
                },
            ),
        ],
        active_repo: Some(RepoId(2)),
        ..AppState::test_default()
    };

    let first = snapshot_repos_from_state(&state);
    let second = snapshot_repos_from_state(&state);

    assert!(Arc::ptr_eq(&first.open_repos, &second.open_repos));
}

#[test]
fn snapshot_excludes_provisional_drop_and_preserves_its_previous_active_tab() {
    clear_session_repos_snapshot_cache();

    let repo_a = PathBuf::from("/tmp/repo-a");
    let repo_b = PathBuf::from("/tmp/repo-b");
    let dropped = PathBuf::from("/tmp/dropped-repo");
    let mut first = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: repo_a.clone(),
        },
    );
    first.last_active_at =
        Some(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1));
    let mut second = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: repo_b.clone(),
        },
    );
    second.last_active_at =
        Some(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2));
    let provisional = RepoState::new_external_drop_opening(
        RepoId(3),
        RepoSpec {
            workdir: dropped.clone(),
        },
        Some(RepoId(1)),
    );
    let mut state = AppState {
        repos: vec![first, second, provisional],
        active_repo: Some(RepoId(3)),
        ..AppState::test_default()
    };

    let pending = snapshot_repos_from_state(&state);
    assert_eq!(
        pending.open_repos.as_ref(),
        &[
            path_storage_key_shared(&repo_a),
            path_storage_key_shared(&repo_b),
        ]
    );
    assert_eq!(pending.active_repo_index, Some(0));

    assert!(state.repos[2].commit_external_drop_open());
    let committed = snapshot_repos_from_state(&state);
    assert_eq!(
        committed.open_repos.as_ref(),
        &[
            path_storage_key_shared(&repo_a),
            path_storage_key_shared(&repo_b),
            path_storage_key_shared(&dropped),
        ]
    );
    assert_eq!(committed.active_repo_index, Some(2));
}

#[test]
fn snapshot_repos_from_state_cache_keeps_dedup_index_for_duplicate_workdirs() {
    let repo_a = PathBuf::from("/tmp/repo-a");
    let mut state = AppState {
        repos: vec![
            RepoState::new_opening(
                RepoId(1),
                RepoSpec {
                    workdir: repo_a.clone(),
                },
            ),
            RepoState::new_opening(RepoId(2), RepoSpec { workdir: repo_a }),
        ],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let first = snapshot_repos_from_state(&state);
    state.active_repo = Some(RepoId(2));
    let second = snapshot_repos_from_state(&state);

    assert!(Arc::ptr_eq(&first.open_repos, &second.open_repos));
    assert_eq!(second.active_repo_index, Some(0));
}

#[test]
fn snapshot_repos_from_state_preserves_first_seen_order_for_repeated_workdirs() {
    clear_session_repos_snapshot_cache();

    let repo_a = PathBuf::from("/tmp/repo-a");
    let repo_b = PathBuf::from("/tmp/repo-b");
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
                    workdir: repo_b.clone(),
                },
            ),
            RepoState::new_opening(
                RepoId(3),
                RepoSpec {
                    workdir: repo_a.clone(),
                },
            ),
        ],
        active_repo: Some(RepoId(3)),
        ..AppState::test_default()
    };

    let snapshot = snapshot_repos_from_state(&state);
    assert_eq!(
        snapshot.open_repos.as_ref(),
        &[
            path_storage_key_shared(&repo_a),
            path_storage_key_shared(&repo_b)
        ]
    );
    assert_eq!(snapshot.active_repo_index, Some(0));
}

#[test]
fn snapshot_repos_from_state_cache_invalidates_when_repo_order_changes() {
    clear_session_repos_snapshot_cache();

    let repo_a = PathBuf::from("/tmp/repo-a");
    let repo_b = PathBuf::from("/tmp/repo-b");
    let mut state = AppState {
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
                    workdir: repo_b.clone(),
                },
            ),
        ],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let first = snapshot_repos_from_state(&state);
    state.repos.swap(0, 1);
    let second = snapshot_repos_from_state(&state);

    assert!(
        !Arc::ptr_eq(&first.open_repos, &second.open_repos),
        "reordering repos should invalidate the cached open-repo slice"
    );
    assert_eq!(
        second.open_repos.as_ref(),
        &[
            path_storage_key_shared(&repo_b),
            path_storage_key_shared(&repo_a)
        ]
    );
    assert_eq!(second.active_repo_index, Some(1));
}

#[test]
fn snapshot_repos_from_state_cache_invalidates_when_repo_spec_changes() {
    clear_session_repos_snapshot_cache();

    let repo_a = PathBuf::from("/tmp/repo-a");
    let repo_b = PathBuf::from("/tmp/repo-b");
    let mut state = AppState {
        repos: vec![RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: repo_a.clone(),
            },
        )],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let first = snapshot_repos_from_state(&state);
    state.repos[0].set_spec(RepoSpec {
        workdir: repo_b.clone(),
    });
    let second = snapshot_repos_from_state(&state);

    assert!(
        !Arc::ptr_eq(&first.open_repos, &second.open_repos),
        "changing the repo spec should invalidate the cached open-repo slice"
    );
    assert_eq!(
        second.open_repos.as_ref(),
        &[path_storage_key_shared(&repo_b)]
    );
    assert_eq!(second.active_repo_index, Some(0));
}

#[test]
fn load_from_path_migrates_v1_files() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-session-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");
    let _ = fs::create_dir_all(&repo_a);
    let _ = fs::create_dir_all(&repo_b);

    persist_to_path(
        &path,
        &UiSessionFileV1 {
            version: SESSION_FILE_VERSION_V1,
            open_repos: vec![path_storage_key(&repo_a), path_storage_key(&repo_b)],
            active_repo: Some(path_storage_key(&repo_b)),
        },
    )
    .expect("persist succeeds");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.open_repos, vec![repo_a, repo_b.clone()]);
    assert_eq!(loaded.active_repo, Some(repo_b));
    assert!(loaded.recent_repos.is_empty());
    assert_eq!(loaded.window_width, None);
    assert_eq!(loaded.date_time_format, None);
}

#[test]
fn load_from_path_migrates_v2_scaled_dimensions_to_design_units() {
    let cases = [
        (100, 280, 420, 222, 111),
        (125, 350, 525, 278, 139),
        (200, 560, 840, 444, 222),
    ];

    for (percent, sidebar_width, details_width, change_tracking_height, untracked_height) in cases {
        let dir = env::temp_dir().join(format!(
            "gitcomet-session-v2-migration-test-{}-{}-{percent}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("session.json");

        persist_to_path(
            &path,
            &UiSessionFile {
                version: SESSION_FILE_VERSION_V2,
                open_repos: Vec::new(),
                active_repo: None,
                sidebar_width: Some(sidebar_width),
                details_width: Some(details_width),
                ui_scale_percent: Some(percent),
                change_tracking_height: Some(change_tracking_height),
                untracked_height: Some(untracked_height),
                ..UiSessionFile::default()
            },
        )
        .expect("persist succeeds");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.ui_scale_percent, Some(percent));
        assert_eq!(loaded.sidebar_width, Some(280));
        assert_eq!(loaded.details_width, Some(420));
        assert_eq!(loaded.change_tracking_height, Some(222));
        assert_eq!(loaded.untracked_height, Some(111));
    }
}

#[test]
fn load_from_path_migrates_v2_scaled_dimensions_without_saved_zoom_as_100_percent() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-session-v2-migration-default-scale-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: SESSION_FILE_VERSION_V2,
            open_repos: Vec::new(),
            active_repo: None,
            sidebar_width: Some(280),
            details_width: Some(420),
            change_tracking_height: Some(222),
            untracked_height: Some(111),
            ..UiSessionFile::default()
        },
    )
    .expect("persist succeeds");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.sidebar_width, Some(280));
    assert_eq!(loaded.details_width, Some(420));
    assert_eq!(loaded.change_tracking_height, Some(222));
    assert_eq!(loaded.untracked_height, Some(111));
}

#[test]
fn persist_recent_repo_round_trips_dedup_and_reorders() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-recent-repos-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");
    let _ = fs::create_dir_all(&repo_a);
    let _ = fs::create_dir_all(&repo_b);

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_recent_repo_to_path(&repo_a, &path).expect("persist first repo");
    persist_recent_repo_to_path(&repo_b, &path).expect("persist second repo");
    persist_recent_repo_to_path(&repo_a, &path).expect("move repo to front");

    // Recents are stored canonicalized so they compare equal to the workdir
    // an open repository carries.
    let canonical = |path: &std::path::Path| {
        gitcomet_core::path_utils::canonicalize_or_original(path.to_path_buf())
    };
    let loaded = load_from_path(&path);
    assert_eq!(
        loaded.recent_repos,
        vec![canonical(&repo_a), canonical(&repo_b)]
    );
}

#[test]
fn history_highlight_commit_chain_round_trips_and_defaults_to_unset() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-highlight-chain-setting-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let session_file = dir.join("session.json");

    // Absent from the file, so the UI applies its own default rather than
    // the setting silently reading as "off".
    assert_eq!(
        load_from_path(&session_file).history_highlight_commit_chain,
        None
    );

    persist_ui_settings_to_path(
        UiSettings {
            history_highlight_commit_chain: Some(false),
            ..UiSettings::default()
        },
        &session_file,
    )
    .expect("persist highlight setting");
    assert_eq!(
        load_from_path(&session_file).history_highlight_commit_chain,
        Some(false)
    );

    persist_ui_settings_to_path(
        UiSettings {
            history_highlight_commit_chain: Some(true),
            ..UiSettings::default()
        },
        &session_file,
    )
    .expect("re-enable highlight setting");
    assert_eq!(
        load_from_path(&session_file).history_highlight_commit_chain,
        Some(true)
    );
}

#[test]
fn remove_recent_repo_drops_matching_entry() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-remove-recent-repo-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");
    let _ = fs::create_dir_all(&repo_a);
    let _ = fs::create_dir_all(&repo_b);

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            recent_repos: Some(vec![path_storage_key(&repo_a), path_storage_key(&repo_b)]),
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    remove_recent_repo_to_path(&repo_b, &path).expect("remove invalid recent repo");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.recent_repos, vec![repo_a]);
}

#[test]
fn persist_pinned_repo_appends_in_pin_order_and_dedupes() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-pinned-repos-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_pinned_repo_to_path(&repo_a, &path).expect("pin first repo");
    persist_pinned_repo_to_path(&repo_b, &path).expect("pin second repo");
    // Re-pinning keeps the original position rather than moving the repo,
    // unlike the recents, which are an MRU list.
    persist_pinned_repo_to_path(&repo_a, &path).expect("re-pin first repo");

    let loaded = load_from_path(&path);
    assert_eq!(
        loaded.pinned_repos,
        vec![repo_a.clone(), repo_b.clone()],
        "re-pinning must not reorder the pin list"
    );

    remove_pinned_repo_to_path(&repo_b, &path).expect("unpin second repo");
    assert_eq!(load_from_path(&path).pinned_repos, vec![repo_a]);
}

/// The stored keys are written trimmed, but a hand-edited file can pad them.
/// A padded copy is still the same repository, so it must not survive the
/// promotion as a second entry.
#[test]
fn persist_recent_repo_collapses_a_padded_duplicate() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-recent-padded-dupe-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");
    let repo = dir.join("repo-padded");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            recent_repos: Some(vec![
                format!("  {}  ", path_storage_key(&repo)),
                "   ".to_owned(),
            ]),
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_recent_repo_to_path(&repo, &path).expect("record repo as recent");

    assert_eq!(
        load_from_path(&path).recent_repos,
        vec![repo],
        "the padded entry and the blank should both be gone"
    );
}

#[test]
fn pinned_repos_survive_the_recent_repository_cap() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-pinned-repo-cap-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");
    let pinned = dir.join("repo-pinned");

    persist_pinned_repo_to_path(&pinned, &path).expect("pin repo");
    persist_recent_repo_to_path(&pinned, &path).expect("record repo as recent");
    for ix in 0..MAX_RECENT_REPOS {
        persist_recent_repo_to_path(&dir.join(format!("repo-{ix}")), &path)
            .expect("push the pinned repo off the recents tail");
    }

    let loaded = load_from_path(&path);
    assert!(
        !loaded.recent_repos.contains(&pinned),
        "the pinned repository should have fallen off the capped recents list"
    );
    assert_eq!(
        loaded.pinned_repos,
        vec![pinned],
        "pins are a separate, uncapped list, so the repository is still reachable"
    );
}

/// A UI holding its own copy of the recents has to be able to apply a bump
/// without re-reading the file, and the copy has to still match the file
/// afterwards — including at the cap, where the file drops its tail.
#[test]
fn promote_recent_repo_matches_the_file_at_the_cap() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-promote-recent-cap-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    // One more repository than the list can hold, so the cap is in play.
    let repos: Vec<PathBuf> = (0..=MAX_RECENT_REPOS)
        .map(|ix| dir.join(format!("repo-{ix}")))
        .collect();
    for repo in &repos {
        persist_recent_repo_to_path(repo, &path).expect("record repo as recent");
    }

    let mut cached = load_from_path(&path).recent_repos;
    assert_eq!(cached.len(), MAX_RECENT_REPOS);

    // The one the cap pushed off comes back to the front, on both sides.
    let evicted = repos[0].clone();
    assert!(!cached.contains(&evicted));
    promote_recent_repo(&mut cached, &evicted);
    persist_recent_repo_to_path(&evicted, &path).expect("re-record the evicted repo");

    assert_eq!(
        cached.len(),
        MAX_RECENT_REPOS,
        "the in-memory list has to honour the same cap the file does"
    );
    assert_eq!(
        cached,
        load_from_path(&path).recent_repos,
        "a promoted cache must match what the next load returns"
    );

    // Re-promoting something already listed moves it without growing the list.
    let already_listed = cached[3].clone();
    promote_recent_repo(&mut cached, &already_listed);
    assert_eq!(cached.len(), MAX_RECENT_REPOS);
    assert_eq!(cached.first(), Some(&already_listed));
    assert_eq!(
        cached
            .iter()
            .filter(|path| **path == already_listed)
            .count(),
        1
    );
}

#[test]
fn persist_ui_settings_round_trips_repo_picker_collapsed_sections() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-picker-collapse-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    assert!(
        load_from_path(&path)
            .repo_picker_collapsed_sections
            .is_empty()
    );

    let collapsed = BTreeSet::from(["open".to_string(), "recently_closed".to_string()]);
    persist_ui_settings_to_path(
        UiSettings {
            repo_picker_collapsed_sections: Some(collapsed.clone()),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist collapsed sections");
    assert_eq!(
        load_from_path(&path).repo_picker_collapsed_sections,
        collapsed
    );

    // An unrelated write must leave the collapse state alone.
    persist_ui_settings_to_path(
        UiSettings {
            repo_picker_sort: Some("name".to_string()),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist unrelated setting");
    assert_eq!(
        load_from_path(&path).repo_picker_collapsed_sections,
        collapsed
    );
}

#[test]
fn remove_recent_repo_drops_entries_written_uncanonicalized() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-remove-recent-legacy-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    let repo = dir.join("repo-a");
    let _ = fs::create_dir_all(&repo);
    let canonical = gitcomet_core::path_utils::canonicalize_or_original(repo.clone());
    // Only meaningful where the temp directory is reached through a symlink
    // (macOS /var -> /private/var); elsewhere the two forms coincide.
    if canonical == repo {
        return;
    }

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            // The uncanonicalized form an older build would have written.
            recent_repos: Some(vec![path_storage_key(&repo)]),
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    remove_recent_repo_to_path(&repo, &path).expect("remove legacy recent repo");

    let loaded = load_from_path(&path);
    assert!(
        loaded.recent_repos.is_empty(),
        "legacy uncanonicalized entry should have been removed, got {:?}",
        loaded.recent_repos
    );
}

/// The mirror of the case above: the caller spells the repository one way
/// and the stored entry spells it another. Matching on the caller's key
/// alone misses it, so removal normalizes the stored side too.
#[test]
fn remove_recent_repo_matches_an_entry_spelled_through_a_symlink() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-remove-recent-normalized-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    let repo = dir.join("repo-a");
    let _ = fs::create_dir_all(&repo);
    let canonical = gitcomet_core::path_utils::canonicalize_or_original(repo.clone());
    // Only meaningful where the temp directory is reached through a symlink
    // (macOS /var -> /private/var); elsewhere the two forms coincide.
    if canonical == repo {
        return;
    }

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            // Stored uncanonicalized, while the caller below passes the
            // canonical form -- so neither of the two keys built from the
            // caller's path matches this string.
            recent_repos: Some(vec![path_storage_key(&repo)]),
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    remove_recent_repo_to_path(&canonical, &path).expect("remove recent repo");

    let loaded = load_from_path(&path);
    assert!(
        loaded.recent_repos.is_empty(),
        "an entry that resolves to the same directory should have been removed, got {:?}",
        loaded.recent_repos
    );
}

/// Storage keys are not paths: a non-UTF-8 workdir is stored hex-encoded
/// behind [`SESSION_PATH_BYTES_PREFIX`], so normalizing an entry has to run
/// it back through [`path_from_storage_key`] first. Reading the encoded key
/// as a literal path makes it look relative, and the entry is skipped.
///
/// Exercised with an encoded key for a *UTF-8* path, which the encoder
/// itself never produces but a hand-edited or older file can hold: APFS
/// rejects the invalid bytes outright, so a genuinely non-UTF-8 directory
/// cannot be created to test against.
#[cfg(unix)]
#[test]
fn remove_recent_repo_normalizes_an_encoded_entry_through_its_decoder() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-remove-recent-encoded-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    let repo = dir.join("repo-a");
    let _ = fs::create_dir_all(&repo);
    let canonical = gitcomet_core::path_utils::canonicalize_or_original(repo.clone());
    // Only meaningful where the two spellings differ (macOS /var -> /private/var):
    // the entry has to need normalizing, not just decoding.
    if canonical == repo {
        return;
    }

    let encoded = format!(
        "{SESSION_PATH_BYTES_PREFIX}{}",
        hex_encode(repo.as_os_str().as_encoded_bytes())
    );
    assert_eq!(
        path_from_storage_key(&encoded),
        repo,
        "the fixture has to decode back to the repository it names"
    );

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            recent_repos: Some(vec![encoded]),
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    remove_recent_repo_to_path(&canonical, &path).expect("remove recent repo");

    let loaded = load_from_path(&path);
    assert!(
        loaded.recent_repos.is_empty(),
        "an encoded entry naming the same directory should have been removed, got {:?}",
        loaded.recent_repos
    );
}

/// Entries a hand-edited file can hold that must never be resolved against
/// the process working directory.
#[test]
fn remove_recent_repo_leaves_relative_entries_alone() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-remove-recent-relative-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            recent_repos: Some(vec![".".to_string(), "../elsewhere".to_string()]),
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    // `.` resolves to whatever directory the test process happens to be in.
    // Removing some unrelated repository must not take it with it.
    remove_recent_repo_to_path(&dir.join("repo-a"), &path).expect("remove recent repo");

    let loaded = load_from_path(&path);
    assert_eq!(
        loaded.recent_repos.len(),
        2,
        "relative entries must survive an unrelated removal, got {:?}",
        loaded.recent_repos
    );
}

#[test]
fn persist_recent_repo_truncates_to_max_entries_and_skips_blank_values() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-recent-repo-truncate-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");
    let repo_new = dir.join("repo-new");

    let mut recent_repos = vec!["   ".to_string()];
    recent_repos
        .extend((0..MAX_RECENT_REPOS).map(|ix| path_storage_key(&dir.join(format!("repo-{ix}")))));

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            recent_repos: Some(recent_repos),
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_recent_repo_to_path(&repo_new, &path).expect("persist latest repo");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.recent_repos.len(), MAX_RECENT_REPOS);
    assert_eq!(loaded.recent_repos.first(), Some(&repo_new));
    assert_eq!(
        loaded.recent_repos.last(),
        Some(&dir.join(format!("repo-{}", MAX_RECENT_REPOS - 2)))
    );
    assert!(
        !loaded
            .recent_repos
            .contains(&dir.join(format!("repo-{}", MAX_RECENT_REPOS - 1)))
    );
    assert!(
        !loaded
            .recent_repos
            .iter()
            .any(|path| path.as_os_str().is_empty())
    );
}

#[test]
fn load_from_path_filters_blank_and_duplicate_recent_repos() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-recent-repo-load-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            recent_repos: Some(vec![
                "   ".to_string(),
                path_storage_key(&repo_a),
                path_storage_key(&repo_a),
                path_storage_key(&repo_b),
                "".to_string(),
            ]),
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.recent_repos, vec![repo_a, repo_b]);
}

#[test]
fn persist_ui_settings_round_trips_repo_sidebar_collapsed_items() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");
    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    let mut repo_sidebar_collapsed_items = BTreeMap::new();
    repo_sidebar_collapsed_items.insert(
        repo_a.clone(),
        BTreeSet::from([
            "section:branches".to_string(),
            "group:local:feature".to_string(),
        ]),
    );
    repo_sidebar_collapsed_items.insert(
        repo_b.clone(),
        BTreeSet::from(["section:worktrees".to_string()]),
    );

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: Some(repo_sidebar_collapsed_items.clone()),
            theme_mode: None,
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: None,
            date_time_format: None,
            timezone: None,
            show_timezone: None,
            change_tracking_view: None,
            diff_scroll_sync: None,
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: None,
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(
        loaded.repo_sidebar_collapsed_items,
        repo_sidebar_collapsed_items
    );
}

#[test]
fn persist_ui_settings_round_trips_repo_sidebar_pinned_branches() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");
    let repo_a = dir.join("repo-a");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    let mut repo_sidebar_pinned_branches = BTreeMap::new();
    repo_sidebar_pinned_branches.insert(
        repo_a.clone(),
        BTreeSet::from([
            "local:main".to_string(),
            "remote:origin/main".to_string(),
            "group:local:feat/nested".to_string(),
            "group:remote:team/origin:feat".to_string(),
        ]),
    );

    persist_ui_settings_to_path(
        UiSettings {
            repo_sidebar_pinned_branches: Some(repo_sidebar_pinned_branches.clone()),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(
        loaded.repo_sidebar_pinned_branches,
        repo_sidebar_pinned_branches
    );
}

#[test]
fn persist_ui_settings_round_trips_date_time_format() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: None,
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: None,
            date_time_format: Some("ymd_hm_utc".to_string()),
            timezone: None,
            show_timezone: None,
            change_tracking_view: None,
            diff_scroll_sync: None,
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: None,
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.date_time_format.as_deref(), Some("ymd_hm_utc"));
}

#[test]
fn persist_ui_settings_round_trips_show_timezone() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: None,
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: Some(false),
            date_time_format: None,
            timezone: None,
            show_timezone: Some(false),
            change_tracking_view: None,
            diff_scroll_sync: None,
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: None,
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.show_timezone, Some(false));
}

#[test]
fn persist_ui_settings_round_trips_font_ligatures() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: None,
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: Some(true),
            date_time_format: None,
            timezone: None,
            show_timezone: None,
            change_tracking_view: None,
            diff_scroll_sync: None,
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: None,
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.use_font_ligatures, Some(true));
}

#[test]
fn persist_ui_settings_round_trips_change_tracking_view() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: None,
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: None,
            date_time_format: None,
            timezone: None,
            show_timezone: None,
            change_tracking_view: Some("split_untracked".to_string()),
            diff_scroll_sync: None,
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: None,
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(
        loaded.change_tracking_view.as_deref(),
        Some("split_untracked")
    );
}

#[test]
fn persist_ui_settings_round_trips_diff_scroll_sync() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: None,
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: None,
            date_time_format: None,
            timezone: None,
            show_timezone: None,
            change_tracking_view: None,
            diff_scroll_sync: Some("horizontal".to_string()),
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: None,
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.diff_scroll_sync.as_deref(), Some("horizontal"));
}

#[test]
fn persist_ui_settings_round_trips_diff_content_mode() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: None,
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: None,
            date_time_format: None,
            timezone: None,
            show_timezone: None,
            change_tracking_view: None,
            diff_scroll_sync: None,
            diff_content_mode: Some("changed_lines_only".to_string()),
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: None,
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(
        loaded.diff_content_mode.as_deref(),
        Some("changed_lines_only")
    );
}

#[test]
fn persist_ui_settings_round_trips_diff_whitespace_mode() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: None,
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: None,
            date_time_format: None,
            timezone: None,
            show_timezone: None,
            change_tracking_view: None,
            diff_scroll_sync: None,
            diff_content_mode: None,
            diff_whitespace_mode: Some("ignore".to_string()),
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: None,
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.diff_whitespace_mode.as_deref(), Some("ignore"));
}

#[test]
fn persist_ui_settings_round_trips_diff_render_settings() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            diff_reveal_whitespace_chars: Some(true),
            diff_word_wrap: Some(true),
            diff_show_line_numbers: Some(false),
            mergetool_show_line_numbers: Some(false),
            mergetool_view_three_way: Some(false),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.diff_reveal_whitespace_chars, Some(true));
    assert_eq!(loaded.diff_word_wrap, Some(true));
    assert_eq!(loaded.diff_show_line_numbers, Some(false));
    assert_eq!(loaded.mergetool_show_line_numbers, Some(false));
    assert_eq!(loaded.mergetool_view_three_way, Some(false));
}

#[test]
fn persist_ui_settings_round_trips_auto_save_file_edits() {
    let dir = unique_session_test_dir("auto-save-file-edits");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    // Absent from the file means "not chosen yet", which the UI reads as off.
    assert_eq!(load_from_path(&path).auto_save_file_edits, None);

    persist_ui_settings_to_path(
        UiSettings {
            auto_save_file_edits: Some(true),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");
    assert_eq!(load_from_path(&path).auto_save_file_edits, Some(true));

    // A later write that says nothing about the toggle must not clear it.
    persist_ui_settings_to_path(
        UiSettings {
            diff_word_wrap: Some(true),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist unrelated ui settings");
    assert_eq!(load_from_path(&path).auto_save_file_edits, Some(true));

    persist_ui_settings_to_path(
        UiSettings {
            auto_save_file_edits: Some(false),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");
    assert_eq!(load_from_path(&path).auto_save_file_edits, Some(false));
}

#[test]
fn persist_ui_settings_round_trips_security_preferences() {
    let dir = unique_session_test_dir("security-preferences");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_ui_settings_to_path(
        UiSettings {
            remote_markdown_image_policy: Some("ask".to_string()),
            allowed_remote_protocols: Some(
                ["https", "ssh", "http"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
            check_for_updates_on_startup: Some(false),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist security preferences");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.remote_markdown_image_policy.as_deref(), Some("ask"));
    assert_eq!(
        loaded.allowed_remote_protocols,
        Some(
            ["http", "https", "ssh"]
                .into_iter()
                .map(str::to_string)
                .collect()
        )
    );
    assert_eq!(loaded.check_for_updates_on_startup, Some(false));
}

#[test]
fn persist_ui_settings_round_trips_change_tracking_heights() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: None,
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: None,
            date_time_format: None,
            timezone: None,
            show_timezone: None,
            change_tracking_view: None,
            diff_scroll_sync: None,
            change_tracking_height: Some(222),
            untracked_height: Some(111),
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: None,
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.change_tracking_height, Some(222));
    assert_eq!(loaded.untracked_height, Some(111));
}

#[test]
fn persist_ui_settings_round_trips_theme_mode() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: Some("dark".to_string()),
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: None,
            date_time_format: None,
            timezone: None,
            show_timezone: None,
            change_tracking_view: None,
            diff_scroll_sync: None,
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: None,
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.theme_mode.as_deref(), Some("dark"));
}

#[test]
fn persist_ui_settings_round_trips_terminal_preferences() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            terminal_external_mode: Some("custom_program".to_string()),
            terminal_external_program: Some("wezterm".to_string()),
            terminal_external_args: Some(vec![
                "start".to_string(),
                "--cwd".to_string(),
                "{cwd}".to_string(),
            ]),
            terminal_action_bar_target: Some("external".to_string()),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(
        loaded.terminal_external_mode.as_deref(),
        Some("custom_program")
    );
    assert_eq!(loaded.terminal_external_program.as_deref(), Some("wezterm"));
    assert_eq!(
        loaded.terminal_external_args,
        Some(vec![
            "start".to_string(),
            "--cwd".to_string(),
            "{cwd}".to_string()
        ])
    );
    assert_eq!(
        loaded.terminal_action_bar_target.as_deref(),
        Some("external")
    );
}

#[test]
fn persist_ui_settings_round_trips_ui_scale_percent() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            ui_scale_percent: Some(125),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.ui_scale_percent, Some(125));
}

#[test]
fn persist_ui_settings_round_trips_empty_custom_git_executable_path() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            repo_sidebar_collapsed_items: None,
            theme_mode: None,
            ui_font_family: None,
            editor_font_family: None,
            use_font_ligatures: None,
            date_time_format: None,
            timezone: None,
            show_timezone: None,
            change_tracking_view: None,
            diff_scroll_sync: None,
            change_tracking_height: None,
            untracked_height: None,
            history_show_author: None,
            history_show_date: None,
            history_show_sha: None,
            git_executable_path: Some(Some(PathBuf::new())),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.git_executable_path, Some(PathBuf::new()));
}

#[test]
fn persist_ui_settings_round_trips_commit_push_after_enabled() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            commit_push_after_enabled: Some(true),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.commit_push_after_enabled, Some(true));
}

#[test]
fn persist_ui_settings_round_trips_fetch_prune_deleted_remote_branches() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-ui-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_to_path(
        &path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_ui_settings_to_path(
        UiSettings {
            fetch_prune_deleted_remote_branches: Some(false),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist remote pruning setting");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.fetch_prune_deleted_remote_branches, Some(false));
}

#[test]
fn v3_repo_prune_preferences_migrate_to_a_conservative_global_preference() {
    let dir = unique_session_test_dir("legacy-repo-fetch-prune-migration");
    let path = dir.join("session.json");
    let legacy_session = serde_json::json!({
        "version": SESSION_FILE_VERSION_V3,
        "open_repos": [],
        "active_repo": null,
        "repo_fetch_prune_deleted_remote_tracking_branches": {
            "/repos/pruning-enabled": true,
            "/repos/pruning-disabled": false
        }
    });
    fs::write(
        &path,
        serde_json::to_vec(&legacy_session).expect("serialize legacy session"),
    )
    .expect("write legacy session");

    let loaded = load_from_path(&path);
    assert_eq!(
        loaded.fetch_prune_deleted_remote_branches,
        Some(false),
        "one saved opt-out must keep pruning disabled after the setting becomes global"
    );

    persist_ui_settings_to_path(
        UiSettings {
            sidebar_collapsed: Some(true),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist an unrelated UI setting");

    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read migrated session"))
            .expect("parse migrated session");
    assert_eq!(
        persisted
            .get("fetch_prune_deleted_remote_branches")
            .and_then(serde_json::Value::as_bool),
        Some(false),
        "the migrated global choice must survive the next session save"
    );
}

#[test]
fn persist_repo_history_scope_round_trips() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-repo-history-scope-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let session_path = dir.join("session.json");

    let repo_a = dir.join("repo-a");
    let _ = fs::create_dir_all(&repo_a);

    persist_to_path(
        &session_path,
        &UiSessionFile {
            version: CURRENT_SESSION_FILE_VERSION,
            open_repos: Vec::new(),
            active_repo: None,
            ..UiSessionFile::default()
        },
    )
    .expect("seed session file");

    persist_repo_history_scope_to_path(&repo_a, LogScope::AllBranches, &session_path)
        .expect("persist repo history scope");

    let loaded = load_repo_history_scope_from_path(&repo_a, &session_path);
    assert_eq!(loaded, Some(LogScope::AllBranches));
}

#[test]
fn persist_repo_history_scope_skips_rewriting_unchanged_value() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-repo-history-scope-noop-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let session_path = dir.join("session.json");
    let repo_a = dir.join("repo-a");
    let _ = fs::create_dir_all(&repo_a);

    persist_repo_history_scope_to_path(&repo_a, LogScope::AllBranches, &session_path)
        .expect("persist repo history scope");

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        let metadata_before = fs::metadata(&session_path).expect("session metadata before");
        let inode_before = metadata_before.ino();

        persist_repo_history_scope_to_path(&repo_a, LogScope::AllBranches, &session_path)
            .expect("persist unchanged repo history scope");

        let metadata_after = fs::metadata(&session_path).expect("session metadata after");
        assert_eq!(
            metadata_after.ino(),
            inode_before,
            "unchanged history scope should not rewrite the session file"
        );
    }

    #[cfg(not(unix))]
    {
        let contents_before = fs::read(&session_path).expect("session bytes before");

        persist_repo_history_scope_to_path(&repo_a, LogScope::AllBranches, &session_path)
            .expect("persist unchanged repo history scope");

        let contents_after = fs::read(&session_path).expect("session bytes after");
        assert_eq!(
            contents_after, contents_before,
            "unchanged history scope should not rewrite the session file"
        );
    }
}

#[test]
fn persist_ui_settings_round_trips_window_controls_mode() {
    let dir = env::temp_dir().join(format!(
        "gitcomet-window-controls-settings-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    persist_ui_settings_to_path(
        UiSettings {
            window_controls_mode: Some("hide".to_string()),
            browser_open_target: Some("new_window".to_string()),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist window controls mode");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.window_controls_mode.as_deref(), Some("hide"));
    assert_eq!(loaded.browser_open_target.as_deref(), Some("new_window"));
}

#[test]
fn saved_workspace_automatic_name_uses_the_first_repository_and_count() {
    let mut workspace = Workspace::new(vec![
        PathBuf::from("/work/alpha"),
        PathBuf::from("/work/beta"),
        PathBuf::from("/work/gamma"),
    ]);
    assert_eq!(workspace.display_name(), "alpha +2");

    // Switching tabs must not rename the workspace: the name sets the width
    // of its title-bar chip, and the repository tabs sit beside it.
    workspace.active_repository = Some(PathBuf::from("/work/beta"));
    assert_eq!(workspace.display_name(), "alpha +2");

    workspace.custom_name = Some("  Client work  ".to_string());
    assert_eq!(workspace.display_name(), "Client work");
}

#[test]
fn v3_open_repositories_migrate_to_one_restorable_workspace() {
    let path = unique_session_test_dir("window-group-v3-migration").join("session.json");
    persist_to_path(
        &path,
        &UiSessionFile {
            version: SESSION_FILE_VERSION_V3,
            open_repos: vec!["/work/alpha".to_string(), "/work/beta".to_string()],
            active_repo: Some("/work/beta".to_string()),
            sidebar_width: Some(280),
            details_width: Some(360),
            sidebar_collapsed: Some(true),
            change_tracking_height: Some(180),
            ..UiSessionFile::default()
        },
    )
    .expect("seed v3 session");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.workspaces.len(), 1);
    let group = &loaded.workspaces[0];
    assert_eq!(group.id, LEGACY_WORKSPACE_ID);
    assert_eq!(
        group.repositories,
        vec![PathBuf::from("/work/alpha"), PathBuf::from("/work/beta")]
    );
    assert_eq!(group.active_repository, Some(PathBuf::from("/work/beta")));
    assert!(group.restore_on_launch);
    assert_eq!(group.layout.sidebar_width, Some(280));
    assert_eq!(group.layout.details_width, Some(360));
    assert!(group.layout.sidebar_collapsed);
    assert_eq!(group.layout.change_tracking_height, Some(180));
}

#[test]
fn legacy_fetch_prune_and_workspaces_migrate_together() {
    for version in [SESSION_FILE_VERSION_V2, SESSION_FILE_VERSION_V3] {
        let path =
            unique_session_test_dir("window-group-fetch-prune-migration").join("session.json");
        persist_to_path(
            &path,
            &serde_json::json!({
                "version": version,
                "open_repos": ["/work/alpha", "/work/beta"],
                "active_repo": "/work/beta",
                "ui_density": "comfortable",
                "repo_fetch_prune_deleted_remote_tracking_branches": {
                    "/work/alpha": true,
                    "/work/beta": false
                }
            }),
        )
        .expect("seed legacy session");

        let loaded = load_from_path(&path);
        assert_eq!(loaded.workspaces.len(), 1);
        assert_eq!(loaded.workspaces[0].id, LEGACY_WORKSPACE_ID);
        assert_eq!(loaded.active_repo, Some(PathBuf::from("/work/beta")));
        assert_eq!(loaded.fetch_prune_deleted_remote_branches, Some(false));
        assert_eq!(loaded.ui_density.as_deref(), Some("comfortable"));

        persist_workspaces_to_path(&loaded.workspaces, &path).expect("persist v4 session");
        assert_eq!(load_from_path(&path), loaded);
    }
}

#[test]
fn multiple_workspaces_round_trip_without_last_writer_loss() {
    let path = unique_session_test_dir("window-group-round-trip").join("session.json");
    let mut group_a = Workspace::new(vec![PathBuf::from("/work/a"), PathBuf::from("/work/b")]);
    group_a.id = WorkspaceId::from_u128(10);
    group_a.custom_name = Some("  Backend  ".to_string());
    group_a.color = Some(WorkspaceColor::Blue);
    group_a.last_activation_order = 7;
    group_a.layout.sidebar_width = Some(260);
    group_a.placement.normal_frame = Some(SavedWindowFrame {
        x: 20,
        y: 30,
        width: 1200,
        height: 800,
    });

    let mut group_b = Workspace::new(vec![PathBuf::from("/work/c")]);
    group_b.id = WorkspaceId::from_u128(11);
    group_b.restore_on_launch = false;
    group_b.last_activation_order = 9;

    persist_workspaces_to_path(&[group_a.clone(), group_b.clone()], &path).expect("persist groups");
    let loaded = load_from_path(&path);

    assert_eq!(loaded.workspaces.len(), 2);
    assert_eq!(loaded.workspaces[0].custom_name.as_deref(), Some("Backend"));
    assert_eq!(loaded.workspaces[0].color, Some(WorkspaceColor::Blue));
    assert_eq!(loaded.workspaces[0].placement, group_a.placement);
    assert_eq!(loaded.workspaces[1], group_b);
    assert_eq!(loaded.open_repos, group_a.repositories);

    persist_ui_settings_to_path(
        UiSettings {
            theme_mode: Some("dark".to_string()),
            ui_density: Some("comfortable".to_string()),
            ui_font_size_px: Some(18),
            editor_font_size_px: Some(15),
            markdown_preview_font_size_px: Some(22),
            window_controls_mode: Some("hide".to_string()),
            browser_open_target: Some("new_window".to_string()),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist unrelated global preference");
    assert_eq!(load_from_path(&path).workspaces, loaded.workspaces);

    persist_repos_snapshot_to_path(
        &SessionReposSnapshot {
            open_repos: Arc::from([Arc::<str>::from("/work/legacy-writer")]),
            active_repo_index: None,
        },
        &path,
    )
    .expect("persist legacy repository projection");
    assert_eq!(load_from_path(&path).workspaces, loaded.workspaces);

    persist_workspaces_to_path(&loaded.workspaces, &path).expect("persist groups again");
    let loaded = load_from_path(&path);
    assert_eq!(loaded.ui_density.as_deref(), Some("comfortable"));
    assert_eq!(loaded.ui_font_size_px, Some(18));
    assert_eq!(loaded.editor_font_size_px, Some(15));
    assert_eq!(loaded.markdown_preview_font_size_px, Some(22));
    assert_eq!(loaded.window_controls_mode.as_deref(), Some("hide"));
    assert_eq!(loaded.browser_open_target.as_deref(), Some("new_window"));
}

#[test]
fn overlapping_workspaces_preserve_independent_membership_and_active_repositories() {
    let path = unique_session_test_dir("overlapping-workspaces").join("session.json");
    let shared = PathBuf::from("/work/shared");
    let other = PathBuf::from("/work/other");
    let mut first = Workspace::new(vec![shared.clone(), other.clone()]);
    first.active_repository = Some(shared.clone());
    first.layout.sidebar_width = Some(240);
    let mut second = Workspace::new(vec![other.clone(), shared.clone()]);
    second.active_repository = Some(other.clone());
    second.layout.sidebar_width = Some(360);

    persist_workspaces_to_path(&[first.clone(), second.clone()], &path).unwrap();
    assert_eq!(
        load_from_path(&path).workspaces,
        vec![first.clone(), second.clone()]
    );

    first.repositories = vec![other.clone()];
    first.active_repository = Some(other);
    persist_workspaces_to_path(&[first.clone(), second.clone()], &path).unwrap();
    assert_eq!(load_from_path(&path).workspaces, vec![first, second]);
}

#[test]
fn closed_groups_remain_saved_but_do_not_project_as_open_repositories() {
    let path = unique_session_test_dir("closed-window-groups").join("session.json");
    let mut group = Workspace::new(vec![PathBuf::from("/work/closed")]);
    group.restore_on_launch = false;
    persist_workspaces_to_path(&[group.clone()], &path).expect("persist closed group");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.workspaces, vec![group]);
    assert!(loaded.open_repos.is_empty());
    assert!(loaded.active_repo.is_none());
}

#[test]
fn first_current_write_preserves_one_v3_backup() {
    let path = unique_session_test_dir("workspace-v3-backup").join("session.json");
    persist_to_path(
        &path,
        &UiSessionFile {
            version: SESSION_FILE_VERSION_V3,
            open_repos: vec!["/work/legacy".to_string()],
            active_repo: Some("/work/legacy".to_string()),
            ..UiSessionFile::default()
        },
    )
    .expect("seed v3 session");
    let original = fs::read(&path).expect("read v3 session");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .expect("simulate a legacy session with public permissions");
    }

    let group = Workspace::new(vec![PathBuf::from("/work/legacy")]);
    persist_workspaces_to_path(&[group], &path).expect("persist workspaces");

    let mut backup_name = path.as_os_str().to_os_string();
    backup_name.push(".v3.bak");
    let backup_path = PathBuf::from(backup_name);
    assert_eq!(fs::read(&backup_path).expect("read v3 backup"), original);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for private_path in [&path, &backup_path] {
            assert_eq!(
                fs::metadata(private_path)
                    .expect("private session file")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600,
            );
        }
    }
}

#[test]
fn empty_customized_workspace_round_trips() {
    let path = unique_session_test_dir("workspace-empty-customized").join("session.json");
    let mut named = Workspace::new(Vec::new());
    named.custom_name = Some("Client work".to_string());
    let mut colored = Workspace::new(Vec::new());
    colored.color = Some(WorkspaceColor::Green);
    let mut themed = Workspace::new(Vec::new());
    themed.theme_mode = Some("tokyo_night".to_string());

    persist_workspaces_to_path(&[named.clone(), colored.clone(), themed.clone()], &path)
        .expect("persist workspaces");
    let loaded = load_from_path(&path);

    assert_eq!(loaded.workspaces, vec![named, colored, themed]);
    assert!(loaded.workspaces.iter().all(Workspace::is_customized));
}

#[test]
fn empty_anonymous_workspace_is_dropped_on_persist() {
    let path = unique_session_test_dir("workspace-empty-anonymous").join("session.json");
    let mut blank_name = Workspace::new(Vec::new());
    blank_name.custom_name = Some("   ".to_string());
    assert!(
        !blank_name.is_customized(),
        "a blank name is not a customization"
    );
    let kept = Workspace::new(vec![PathBuf::from("/work/a")]);

    persist_workspaces_to_path(
        &[Workspace::new(Vec::new()), blank_name, kept.clone()],
        &path,
    )
    .expect("persist workspaces");

    assert_eq!(load_from_path(&path).workspaces, vec![kept]);
}

#[test]
fn legacy_projection_skips_empty_workspaces() {
    let path = unique_session_test_dir("workspace-projection-empty").join("session.json");
    let mut with_repos = Workspace::new(vec![PathBuf::from("/work/a")]);
    with_repos.last_activation_order = 1;
    with_repos.layout.sidebar_width = Some(300);
    let mut empty = Workspace::new(Vec::new());
    empty.custom_name = Some("Later".to_string());
    // Most recently activated, so it would win the election without the filter.
    empty.last_activation_order = 2;

    persist_workspaces_to_path(&[with_repos.clone(), empty], &path).expect("persist workspaces");
    let written: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read session")).expect("parse session");
    assert_eq!(written["open_repos"], serde_json::json!(["/work/a"]));

    let loaded = load_from_path(&path);
    assert_eq!(loaded.open_repos, with_repos.repositories);
    assert_eq!(loaded.sidebar_width, Some(300));
}

#[test]
fn theme_mode_and_timestamps_round_trip() {
    let path = unique_session_test_dir("workspace-theme-timestamps").join("session.json");
    let mut workspace = Workspace::new(vec![PathBuf::from("/work/a")]);
    assert!(
        workspace.created_at.is_some(),
        "new workspaces record creation time"
    );
    workspace.theme_mode = Some("  sunset_veil  ".to_string());
    workspace.last_opened_at = Some(1_800_000_000);
    workspace.placement.tiled = Some(SavedWindowTiling {
        left: true,
        top: true,
        bottom: true,
        right: false,
    });

    persist_workspaces_to_path(std::slice::from_ref(&workspace), &path)
        .expect("persist workspaces");
    let loaded = load_from_path(&path).workspaces.remove(0);

    assert_eq!(loaded.theme_mode.as_deref(), Some("sunset_veil"));
    assert_eq!(loaded.created_at, workspace.created_at);
    assert_eq!(loaded.last_opened_at, Some(1_800_000_000));
    assert_eq!(loaded.placement.tiled, workspace.placement.tiled);
}

#[test]
fn v4_window_groups_key_loads_as_workspaces_and_rewrites_as_v5() {
    let path = unique_session_test_dir("workspace-v4-migration").join("session.json");
    fs::create_dir_all(path.parent().expect("session dir")).expect("create session dir");
    let id = WorkspaceId::new();
    let v4 = serde_json::json!({
        "version": 4,
        "window_groups": [{
            "id": id,
            "custom_name": "Client work",
            "color": "blue",
            "repositories": ["/work/alpha"],
            "active_repository": "/work/alpha",
            "restore_on_launch": true,
            "last_activation_order": 3,
            "layout": {"sidebar_width": null, "details_width": null, "sidebar_collapsed": false,
                       "change_tracking_height": null, "untracked_height": null},
            "placement": {"normal_frame": null, "captured_visible_frame": null,
                          "display_id": null, "state": "windowed"}
        }],
        "open_repos": ["/work/alpha"],
        "active_repo": "/work/alpha"
    });
    let original = serde_json::to_vec(&v4).expect("encode v4");
    fs::write(&path, &original).expect("seed v4 session");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.workspaces.len(), 1, "the old key must still load");
    let workspace = loaded.workspaces[0].clone();
    assert_eq!(workspace.id, id);
    assert_eq!(workspace.display_name(), "Client work");
    assert_eq!(workspace.color, Some(WorkspaceColor::Blue));

    persist_workspaces_to_path(&[workspace], &path).expect("persist workspaces");
    let written: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read v5")).expect("parse v5");
    assert_eq!(written["version"], 5);
    assert!(written.get("window_groups").is_none());
    assert_eq!(written["workspaces"][0]["custom_name"], "Client work");

    let mut backup_name = path.as_os_str().to_os_string();
    backup_name.push(".v4.bak");
    assert_eq!(
        fs::read(PathBuf::from(backup_name)).expect("v4 backup"),
        original
    );
}

#[test]
fn persist_ui_settings_round_trips_file_browser_follow_selected_commit() {
    let dir = unique_session_test_dir("file-browser-follow-selected-commit");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("session.json");

    // Absent from the file means "not chosen yet", which the UI reads as on.
    assert_eq!(
        load_from_path(&path).file_browser_follow_selected_commit,
        None
    );

    persist_ui_settings_to_path(
        UiSettings {
            file_browser_follow_selected_commit: Some(false),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist ui settings");
    assert_eq!(
        load_from_path(&path).file_browser_follow_selected_commit,
        Some(false)
    );

    // A later write that says nothing about the toggle must not clear it.
    persist_ui_settings_to_path(
        UiSettings {
            diff_word_wrap: Some(true),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("persist unrelated ui settings");
    assert_eq!(
        load_from_path(&path).file_browser_follow_selected_commit,
        Some(false)
    );
}

/// The layer stores density as an opaque string, so a value this build has never
/// heard of must survive a write/read cycle intact rather than being dropped or
/// normalised — that is what lets a newer build's choice come back.
#[test]
fn an_unknown_density_survives_the_session_round_trip() {
    let path = unique_session_test_dir("density_forward_compat").join("session.json");

    for density in ["compact", "comfortable", "spacious", "something-newer"] {
        persist_ui_settings_to_path(
            UiSettings {
                ui_density: Some(density.into()),
                ..UiSettings::default()
            },
            &path,
        )
        .unwrap();

        assert_eq!(load_from_path(&path).ui_density.as_deref(), Some(density));
    }
}

#[test]
fn appearance_settings_round_trip_and_partial_writes_preserve_independent_sizes() {
    let path = unique_session_test_dir("appearance").join("session.json");
    let default = load_from_path(&path);
    assert_eq!(default.ui_density, None);
    assert_eq!(default.ui_font_size_px, None);
    persist_ui_settings_to_path(
        UiSettings {
            ui_density: Some("comfortable".into()),
            ui_font_size_px: Some(18),
            editor_font_size_px: Some(15),
            markdown_preview_font_size_px: Some(22),
            ..UiSettings::default()
        },
        &path,
    )
    .unwrap();
    persist_ui_settings_to_path(
        UiSettings {
            editor_font_size_px: Some(17),
            ..UiSettings::default()
        },
        &path,
    )
    .unwrap();
    let loaded = load_from_path(&path);
    assert_eq!(loaded.ui_density.as_deref(), Some("comfortable"));
    assert_eq!(loaded.ui_font_size_px, Some(18));
    assert_eq!(loaded.editor_font_size_px, Some(17));
    assert_eq!(loaded.markdown_preview_font_size_px, Some(22));
}

/// A pre-v5 build discards a v5 file and saves its own Compact default, which
/// must not read back as the user's choice.
#[test]
fn pre_v5_compact_density_is_the_old_default_not_a_choice() {
    let seed = |label: &str, version: u32, density: &str| {
        let path = unique_session_test_dir(label).join("session.json");
        persist_to_path(
            &path,
            &UiSessionFile {
                version,
                ui_density: Some(density.to_string()),
                ..UiSessionFile::default()
            },
        )
        .unwrap();
        path
    };

    let v3_compact = seed("density-v3-compact", SESSION_FILE_VERSION_V3, "compact");
    assert_eq!(load_from_path(&v3_compact).ui_density, None);
    persist_ui_settings_to_path(
        UiSettings {
            ui_font_size_px: Some(15),
            ..UiSettings::default()
        },
        &v3_compact,
    )
    .unwrap();
    assert_eq!(
        load_from_path(&v3_compact).ui_density,
        None,
        "an unrelated write must not carry the old default into a v5 file"
    );

    for (version, density) in [
        (SESSION_FILE_VERSION_V3, "comfortable"),
        (SESSION_FILE_VERSION_V4, "spacious"),
        (SESSION_FILE_VERSION_V5, "compact"),
    ] {
        let path = seed(&format!("density-v{version}-{density}"), version, density);
        assert_eq!(load_from_path(&path).ui_density.as_deref(), Some(density));
    }
}

fn seed_v3_session(label: &str, open_repos: &[&str]) -> PathBuf {
    let path = unique_session_test_dir(label).join("session.json");
    persist_to_path(
        &path,
        &UiSessionFile {
            version: SESSION_FILE_VERSION_V3,
            open_repos: open_repos.iter().map(|repo| repo.to_string()).collect(),
            active_repo: open_repos.first().map(|repo| repo.to_string()),
            ..UiSessionFile::default()
        },
    )
    .expect("seed v3 session");
    path
}

fn repos_snapshot(open_repos: &[&str]) -> SessionReposSnapshot {
    SessionReposSnapshot {
        open_repos: open_repos
            .iter()
            .map(|repo| Arc::<str>::from(*repo))
            .collect(),
        active_repo_index: (!open_repos.is_empty()).then_some(0),
    }
}

// A store persisting its now-empty repo list after the UI saved a renamed,
// emptied workspace must not drop that workspace from disk.
#[test]
fn store_repo_snapshot_keeps_an_empty_customized_legacy_workspace() {
    let path = seed_v3_session("workspace-store-keeps-customized", &["/work/alpha"]);
    let mut workspace = load_from_path(&path).workspaces.remove(0);
    assert_eq!(workspace.id, LEGACY_WORKSPACE_ID);
    workspace.custom_name = Some("Client work".to_string());
    workspace.repositories.clear();
    workspace.active_repository = None;
    persist_workspaces_to_path(std::slice::from_ref(&workspace), &path).expect("UI persist");

    persist_repos_snapshot_to_path(&repos_snapshot(&[]), &path).expect("store persist");

    assert_eq!(load_from_path(&path).workspaces, vec![workspace]);
}

// Once the UI has saved workspaces, another store (a second window, or a
// mergetool process) must not rewrite their repositories.
#[test]
fn store_repo_snapshot_does_not_rewrite_workspaces_the_ui_saved() {
    let path = seed_v3_session("workspace-store-no-rewrite", &["/work/alpha", "/work/beta"]);
    let workspaces = load_from_path(&path).workspaces;
    persist_workspaces_to_path(&workspaces, &path).expect("UI persist");

    persist_repos_snapshot_to_path(&repos_snapshot(&["/work/gamma"]), &path)
        .expect("second store persist");

    assert_eq!(load_from_path(&path).workspaces, workspaces);
}

// What the old legacy sync was for: until the UI saves workspaces, the
// store's repo list is what loads, even after an unrelated writer.
#[test]
fn store_repo_snapshot_is_what_loads_until_the_ui_saves_workspaces() {
    let path = seed_v3_session(
        "workspace-store-owns-until-ui",
        &["/work/alpha", "/work/beta"],
    );
    persist_ui_settings_to_path(
        UiSettings {
            theme_mode: Some("dark".to_string()),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("unrelated writer");

    persist_repos_snapshot_to_path(&repos_snapshot(&["/work/gamma"]), &path)
        .expect("store persist");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.open_repos, vec![PathBuf::from("/work/gamma")]);
    assert_eq!(loaded.workspaces.len(), 1);
    assert_eq!(loaded.workspaces[0].id, LEGACY_WORKSPACE_ID);
    assert_eq!(
        loaded.workspaces[0].repositories,
        vec![PathBuf::from("/work/gamma")]
    );
    assert_eq!(loaded.theme_mode.as_deref(), Some("dark"));
}

// One unknown or broken workspace entry must not reset the whole session:
// unknown values degrade and an unparseable entry is dropped on its own.
#[test]
fn unknown_or_broken_workspace_entries_do_not_reset_the_session() {
    let path = unique_session_test_dir("workspace-lenient").join("session.json");
    let kept_id = WorkspaceId::from_u128(21);
    let sparse_id = WorkspaceId::from_u128(22);
    let session = serde_json::json!({
        "version": CURRENT_SESSION_FILE_VERSION,
        "ui_density": "comfortable",
        "theme_mode": "dark",
        "open_repos": ["/work/alpha"],
        "active_repo": "/work/alpha",
        "workspaces": [
            {
                "id": kept_id,
                "custom_name": "Client work",
                "color": "chartreuse",
                "repositories": ["/work/alpha"],
                "active_repository": "/work/alpha",
                "restore_on_launch": true,
                "last_activation_order": 3,
                "placement": {
                    "normal_frame": {"x": 1, "y": 2, "width": 900, "height": 700},
                    "captured_visible_frame": {"x": 0},
                    "state": "minimized",
                    "added_later": 1
                },
                "added_later": true
            },
            {"id": "not-a-uuid", "repositories": ["/work/broken"]},
            {"id": sparse_id, "repositories": ["/work/beta"]}
        ]
    });
    fs::write(&path, serde_json::to_vec(&session).expect("encode")).expect("seed session");

    let loaded = load_from_path(&path);
    assert_eq!(loaded.ui_density.as_deref(), Some("comfortable"));
    assert_eq!(loaded.theme_mode.as_deref(), Some("dark"));
    assert_eq!(
        loaded
            .workspaces
            .iter()
            .map(|workspace| workspace.id)
            .collect::<Vec<_>>(),
        vec![kept_id, sparse_id]
    );
    let kept = &loaded.workspaces[0];
    assert_eq!(kept.custom_name.as_deref(), Some("Client work"));
    assert_eq!(kept.color, None);
    assert_eq!(kept.last_activation_order, 3);
    assert_eq!(kept.layout, WorkspaceLayout::default());
    assert_eq!(kept.placement.state, SavedWindowState::Windowed);
    assert_eq!(kept.placement.captured_visible_frame, None);
    assert_eq!(
        kept.placement.normal_frame,
        Some(SavedWindowFrame {
            x: 1,
            y: 2,
            width: 900,
            height: 700,
        })
    );
    let sparse = &loaded.workspaces[1];
    assert_eq!(sparse.repositories, vec![PathBuf::from("/work/beta")]);
    assert!(
        sparse.restore_on_launch,
        "a sparse entry restores like a new one"
    );
}

// The backup check trusts the version each write just loaded, so a file an
// older build rewrote after this process went current still gets backed up.
#[test]
fn older_build_rewrite_after_a_current_write_is_still_backed_up() {
    let path = unique_session_test_dir("workspace-backup-after-downgrade").join("session.json");
    persist_ui_settings_to_path(
        UiSettings {
            theme_mode: Some("dark".to_string()),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("current write");
    let older = br#"{"version":3,"open_repos":["/work/legacy"],"active_repo":null}"#;
    fs::write(&path, older).expect("older build write");

    persist_ui_settings_to_path(
        UiSettings {
            theme_mode: Some("light".to_string()),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("current write");

    let mut backup_name = path.as_os_str().to_os_string();
    backup_name.push(".v3.bak");
    assert_eq!(
        fs::read(PathBuf::from(backup_name)).expect("v3 backup"),
        older
    );
}

/// Per-write session persist cost on a typical and a heavy session file.
/// `cargo test -p gitcomet-state --lib timing_session_persist -- --ignored --nocapture`
#[test]
#[ignore = "timing probe"]
fn timing_session_persist_cost_by_file_size() {
    const ITERATIONS: u32 = 400;
    for repo_count in [10_usize, 300] {
        let path = unique_session_test_dir("timing-session-persist").join("session.json");
        let repos: Vec<String> = (0..repo_count)
            .map(|ix| format!("/home/user/src/team-{:02}/repository-{ix:04}", ix % 17))
            .collect();
        let per_repo_sets = || {
            repos
                .iter()
                .map(|repo| {
                    let items = (0..8).map(|ix| format!("refs/heads/feature/item-{ix}"));
                    (repo.clone(), items.collect::<BTreeSet<_>>())
                })
                .collect::<BTreeMap<_, _>>()
        };
        let workspaces = repos
            .chunks(5)
            .take(20)
            .enumerate()
            .map(|(ix, chunk)| WorkspaceFile {
                id: WorkspaceId::from_u128(ix as u128 + 1),
                custom_name: Some(format!("Workspace {ix}")),
                color: Some(WorkspaceColor::Blue),
                repositories: chunk.to_vec(),
                active_repository: chunk.first().cloned(),
                restore_on_launch: ix % 3 == 0,
                last_activation_order: ix as u64,
                layout: WorkspaceLayout::default(),
                placement: PortableWindowPlacement {
                    normal_frame: Some(SavedWindowFrame {
                        x: 10,
                        y: 20,
                        width: 1400,
                        height: 900,
                    }),
                    ..PortableWindowPlacement::default()
                },
                theme_mode: None,
                created_at: Some(1_800_000_000),
                last_opened_at: Some(1_800_000_000),
            })
            .collect();
        persist_to_path(
            &path,
            &UiSessionFile {
                version: CURRENT_SESSION_FILE_VERSION,
                workspaces: Some(workspaces),
                open_repos: repos.iter().take(5).cloned().collect(),
                recent_repos: Some(repos.iter().take(MAX_RECENT_REPOS).cloned().collect()),
                pinned_repos: Some(repos.iter().take(20).cloned().collect()),
                repo_sidebar_collapsed_items: Some(per_repo_sets()),
                repo_sidebar_pinned_branches: Some(per_repo_sets()),
                repo_history_modes: Some(
                    repos
                        .iter()
                        .map(|repo| (repo.clone(), HistoryModeSetting::FirstParent))
                        .collect(),
                ),
                repo_history_scopes: Some(
                    repos
                        .iter()
                        .map(|repo| (repo.clone(), HistoryScopeSetting::AllBranches))
                        .collect(),
                ),
                ..UiSessionFile::default()
            },
        )
        .expect("seed session");
        let bytes = fs::metadata(&path).expect("session metadata").len();
        let contents = fs::read(&path).expect("read session");

        let started = std::time::Instant::now();
        for ix in 0..ITERATIONS {
            persist_ui_settings_to_path(
                UiSettings {
                    sidebar_width: Some(200 + ix),
                    ..UiSettings::default()
                },
                &path,
            )
            .expect("persist settings");
        }
        let settings_write = started.elapsed() / ITERATIONS;

        let snapshot = repos_snapshot(&["/home/user/src/a", "/home/user/src/b"]);
        let started = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            persist_repos_snapshot_to_path(&snapshot, &path).expect("persist snapshot");
        }
        let snapshot_write = started.elapsed() / ITERATIONS;

        let started = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            preserve_previous_version_session_backup(&path, &contents).expect("backup check");
        }
        let backup_check = started.elapsed() / ITERATIONS;

        println!(
            "session persist: repos={repo_count} bytes={bytes} settings_write={settings_write:?} \
             snapshot_write={snapshot_write:?} backup_check={backup_check:?}"
        );
    }
}

// The focused mergetool reads back its own size: neither a workspace frame
// nor a running main app's workspace or settings writes may replace it.
#[test]
fn mergetool_window_size_survives_workspace_frames_and_main_app_writes() {
    let path = unique_session_test_dir("mergetool-window-size").join("session.json");
    let mut workspace = Workspace::new(vec![PathBuf::from("/work/alpha")]);
    workspace.placement.normal_frame = Some(SavedWindowFrame {
        x: 0,
        y: 0,
        width: 1200,
        height: 800,
    });
    persist_workspaces_to_path(std::slice::from_ref(&workspace), &path).expect("UI persist");
    persist_mergetool_window_size_to_path(700, 500, &path).expect("mergetool persist");
    persist_workspaces_to_path(std::slice::from_ref(&workspace), &path).expect("UI persist");
    persist_ui_settings_to_path(
        UiSettings {
            window_width: Some(1300),
            window_height: Some(850),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("main window settings persist");

    let loaded = load_from_path(&path);
    assert_eq!(
        (
            loaded.mergetool_window_width,
            loaded.mergetool_window_height
        ),
        (Some(700), Some(500))
    );
    assert_eq!(
        (loaded.window_width, loaded.window_height),
        (Some(1200), Some(800)),
        "normal windows keep the workspace frame"
    );
}

// Before the mergetool saves its own size it opens at the shared legacy one.
#[test]
fn mergetool_window_size_falls_back_to_the_legacy_size() {
    let path = unique_session_test_dir("mergetool-window-fallback").join("session.json");
    persist_ui_settings_to_path(
        UiSettings {
            window_width: Some(900),
            window_height: Some(600),
            ..UiSettings::default()
        },
        &path,
    )
    .expect("legacy size persist");

    let loaded = load_from_path(&path);
    assert_eq!(
        (
            loaded.mergetool_window_width,
            loaded.mergetool_window_height
        ),
        (Some(900), Some(600))
    );
}
