use super::super::*;
use gitcomet_core::domain::HistoryMode;
use serde_json::json;

fn session_path(label: &str) -> PathBuf {
    let dir = env::temp_dir().join(format!(
        "gitcomet-extension-namespace-test-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir.join("session.json")
}

#[test]
fn namespaces_survive_every_ordinary_writer() {
    let path = session_path("writers");
    persist_extension_namespace_to_path("com.example.a", Some(json!({"open": true})), &path)
        .unwrap();
    persist_extension_namespace_to_path("com.example.b", Some(json!([1, 2, 3])), &path).unwrap();

    let repo = Path::new("/repos/a");
    persist_ui_settings_to_path(
        UiSettings {
            ui_scale_percent: Some(125),
            ..Default::default()
        },
        &path,
    )
    .unwrap();
    persist_mergetool_window_size_to_path(900, 700, &path).unwrap();
    persist_recent_repo_to_path(repo, &path).unwrap();
    remove_recent_repo_to_path(repo, &path).unwrap();
    persist_pinned_repo_to_path(repo, &path).unwrap();
    remove_pinned_repo_to_path(repo, &path).unwrap();
    persist_repo_history_mode_to_path(repo, HistoryMode::AllBranches, &path).unwrap();
    persist_survey_prompt_opened_to_path(&path, "survey", 1).unwrap();
    persist_survey_prompt_postponed_to_path(&path, "survey", 60, 2).unwrap();
    persist_repos_snapshot_to_path(
        &SessionReposSnapshot {
            open_repos: Arc::from(vec![Arc::<str>::from("/repos/a")]),
            active_repo_index: Some(0),
        },
        &path,
    )
    .unwrap();
    persist_workspaces_to_path(&[Workspace::new(vec![repo.to_path_buf()])], &path).unwrap();

    assert_eq!(
        extension_namespace_from_path("com.example.a", &path),
        Some(json!({"open": true}))
    );
    assert_eq!(
        extension_namespace_from_path("com.example.b", &path),
        Some(json!([1, 2, 3]))
    );
    assert_eq!(load_from_path(&path).ui_scale_percent, Some(125));
}

/// A writer that loaded earlier cannot drop a namespace written since: every
/// write re-reads the file under the lock.
#[test]
fn a_stale_reader_does_not_undo_an_interleaved_namespace_write() {
    let path = session_path("interleaved");
    persist_ui_settings_to_path(
        UiSettings {
            ui_scale_percent: Some(110),
            ..Default::default()
        },
        &path,
    )
    .unwrap();
    let stale = load_from_path(&path);
    persist_extension_namespace_to_path("com.example.a", Some(json!("fresh")), &path).unwrap();
    persist_ui_settings_to_path(
        UiSettings {
            ui_scale_percent: stale.ui_scale_percent.map(|percent| percent + 10),
            ..Default::default()
        },
        &path,
    )
    .unwrap();
    assert_eq!(
        extension_namespace_from_path("com.example.a", &path),
        Some(json!("fresh"))
    );
    assert_eq!(load_from_path(&path).ui_scale_percent, Some(120));
}

#[test]
fn workspace_namespaces_survive_replacement_and_reload() {
    let path = session_path("workspaces");
    let mut workspace = Workspace::new(vec![PathBuf::from("/repos/a")]);
    workspace
        .extensions
        .set("com.example.a", Some(json!({"panel": "open"})))
        .unwrap();
    persist_workspaces_to_path(std::slice::from_ref(&workspace), &path).unwrap();
    persist_ui_settings_to_path(
        UiSettings {
            ui_scale_percent: Some(90),
            ..Default::default()
        },
        &path,
    )
    .unwrap();

    let loaded = load_from_path(&path).workspaces;
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].extensions.get("com.example.a"),
        Some(&json!({"panel": "open"}))
    );
    // The UI rewrites the whole list from its records; the record carries it.
    persist_workspaces_to_path(&loaded, &path).unwrap();
    let reloaded = load_from_path(&path).workspaces;
    assert_eq!(reloaded[0].extensions, workspace.extensions);
}

#[test]
fn old_session_versions_load_and_upgrade_with_a_namespace() {
    let path = session_path("old-version");
    fs::write(
        &path,
        r#"{"version":3,"open_repos":["/repos/a"],"active_repo":"/repos/a","ui_scale_percent":130}"#,
    )
    .unwrap();
    assert_eq!(extension_namespace_from_path("com.example.a", &path), None);
    persist_extension_namespace_to_path("com.example.a", Some(json!(1)), &path).unwrap();
    let session = load_from_path(&path);
    assert_eq!(session.ui_scale_percent, Some(130));
    assert_eq!(
        extension_namespace_from_path("com.example.a", &path),
        Some(json!(1))
    );
}

#[test]
fn a_malformed_namespace_store_leaves_settings_readable() {
    let path = session_path("malformed");
    fs::write(
        &path,
        r#"{"version":5,"open_repos":[],"active_repo":null,"ui_scale_percent":140,"extensions":"garbage","workspaces":[{"id":"00000000-0000-0000-0000-000000000009","extensions":[1,2]}]}"#,
    )
    .unwrap();
    let session = load_from_path(&path);
    assert_eq!(session.ui_scale_percent, Some(140));
    assert_eq!(extension_namespace_from_path("com.example.a", &path), None);
    // An ordinary write keeps the unreadable value verbatim...
    persist_ui_settings_to_path(
        UiSettings {
            ui_scale_percent: Some(150),
            ..Default::default()
        },
        &path,
    )
    .unwrap();
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(raw["extensions"], json!("garbage"));
    // ...and an explicit namespace write replaces it.
    persist_extension_namespace_to_path("com.example.a", Some(json!(true)), &path).unwrap();
    assert_eq!(
        extension_namespace_from_path("com.example.a", &path),
        Some(json!(true))
    );
    assert_eq!(load_from_path(&path).ui_scale_percent, Some(150));
}

#[test]
fn an_oversized_namespace_is_refused_without_writing() {
    let path = session_path("oversized");
    persist_extension_namespace_to_path("com.example.a", Some(json!("kept")), &path).unwrap();
    let before = fs::read(&path).unwrap();
    let huge = json!("x".repeat(MAX_EXTENSION_NAMESPACE_BYTES));
    let error = persist_extension_namespace_to_path("com.example.a", Some(huge), &path)
        .expect_err("over the limit");
    assert!(matches!(error, ExtensionNamespaceError::TooLarge { .. }));
    assert_eq!(fs::read(&path).unwrap(), before);

    let mut namespaces = ExtensionNamespaces::default();
    assert!(
        namespaces
            .set("a", Some(json!("x".repeat(MAX_EXTENSION_NAMESPACE_BYTES))))
            .is_err()
    );
    assert!(namespaces.is_empty());
}

#[test]
fn clearing_the_last_namespace_removes_the_store() {
    let path = session_path("clear");
    persist_extension_namespace_to_path("com.example.a", Some(json!(1)), &path).unwrap();
    persist_extension_namespace_to_path("com.example.a", None, &path).unwrap();
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(raw.get("extensions").is_none(), "{raw}");
}
