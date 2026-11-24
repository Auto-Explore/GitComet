use super::*;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let dir = env::temp_dir().join(format!(
        "gitcomet-external-editor-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

// On Windows the detector only considers files with an executable
// extension, so tests must create the platform-appropriate launcher name.
fn executable_name(name: &str) -> String {
    #[cfg(windows)]
    {
        if Path::new(name).extension().is_some() {
            name.to_string()
        } else {
            format!("{name}.exe")
        }
    }
    #[cfg(not(windows))]
    {
        name.to_string()
    }
}

fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, b"").expect("create file");
}

fn touch_executable(path: &Path) {
    touch(path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)
            .expect("read file metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("mark file executable");
    }
}

fn ids(editors: &[DetectedExternalEditor]) -> Vec<String> {
    editors.iter().map(|editor| editor.id.clone()).collect()
}

#[test]
fn detects_editors_from_path_and_dedupes_by_id_and_path() {
    let dir = temp_dir("path");
    let code = dir.join(executable_name("code"));
    touch_executable(&code);
    let env = ExternalEditorDetectionEnv {
        path_dirs: vec![dir.clone(), dir],
        ..ExternalEditorDetectionEnv::default()
    };

    let editors = detect_external_editors_with_env(&env);

    assert_eq!(ids(&editors), vec!["vscode"]);
    assert_eq!(editors[0].path, code);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn does_not_detect_xed_as_xcode_outside_macos() {
    let dir = temp_dir("xed-non-macos");
    touch_executable(&dir.join("xed"));
    let env = ExternalEditorDetectionEnv {
        path_dirs: vec![dir],
        ..ExternalEditorDetectionEnv::default()
    };

    let editors = detect_external_editors_with_env(&env);

    assert!(
        editors.iter().all(|editor| editor.id != "xcode"),
        "expected Linux xed to avoid Xcode detection"
    );
}

#[cfg(unix)]
#[test]
fn ignores_non_executable_path_program_candidates_on_unix() {
    let dir = temp_dir("path-non-executable");
    touch(&dir.join("code"));
    touch_executable(&dir.join("vim"));
    touch(&dir.join("xterm"));
    let env = ExternalEditorDetectionEnv {
        path_dirs: vec![dir],
        ..ExternalEditorDetectionEnv::default()
    };

    let editors = detect_external_editors_with_env(&env);

    assert!(
        editors.iter().all(|editor| editor.id != "vscode"),
        "expected non-executable `code` PATH file to be ignored"
    );
    assert!(
        editors.iter().all(|editor| editor.id != "vim-terminal"),
        "expected non-executable terminal launcher to be ignored"
    );
}

#[test]
fn detects_jetbrains_toolbox_paths() {
    let root = temp_dir("toolbox");
    let idea = root.join("IDEA-U/ch-0/251.1/bin/idea.sh");
    touch(&idea);
    let env = ExternalEditorDetectionEnv {
        jetbrains_toolbox_dirs: vec![root],
        ..ExternalEditorDetectionEnv::default()
    };

    let editors = detect_external_editors_with_env(&env);

    assert!(editors.iter().any(|editor| {
        editor.id == "jetbrains-idea" && editor.label == "IntelliJ IDEA" && editor.path == idea
    }));
}

#[test]
fn detects_jetbrains_toolbox_paths_without_a_channel_directory() {
    let root = temp_dir("toolbox-no-channel");
    let clion = root.join("CLion/2026.1/bin/clion64.exe");
    touch(&clion);
    let env = ExternalEditorDetectionEnv {
        jetbrains_toolbox_dirs: vec![root],
        ..ExternalEditorDetectionEnv::default()
    };

    let editors = detect_external_editors_with_env(&env);

    assert!(editors.iter().any(|editor| {
        editor.id == "jetbrains-clion" && editor.label == "CLion" && editor.path == clion
    }));
}

#[test]
fn jetbrains_probe_does_not_descend_into_install_subtrees() {
    let root = temp_dir("toolbox-no-deep-walk");
    let fake = root.join("IDEA-U/ch-0/251.1/plugins/fake/bin/idea.sh");
    touch(&fake);
    fs::create_dir_all(root.join("IDEA-U/ch-0/251.1/bin")).expect("create install bin");
    let env = ExternalEditorDetectionEnv {
        jetbrains_toolbox_dirs: vec![root],
        ..ExternalEditorDetectionEnv::default()
    };

    let editors = detect_external_editors_with_env(&env);

    assert!(editors.iter().all(|editor| editor.path != fake));
}

#[test]
fn detects_jetbrains_standalone_install_paths() {
    let root = temp_dir("jetbrains-standalone");
    let rustrover = root.join("RustRover 2026.1.2/bin/rustrover64.exe");
    touch(&rustrover);
    let env = ExternalEditorDetectionEnv {
        jetbrains_install_dirs: vec![root],
        ..ExternalEditorDetectionEnv::default()
    };

    let editors = detect_external_editors_with_env(&env);

    assert!(editors.iter().any(|editor| {
        editor.id == "jetbrains-rustrover"
            && editor.label == "RustRover"
            && editor.path == rustrover
    }));
}

#[cfg(windows)]
#[test]
fn windows_ignores_extensionless_launcher_next_to_exe() {
    let dir = temp_dir("windows-extensionless");
    touch(&dir.join("zed"));
    touch(&dir.join("zed.exe"));
    let env = ExternalEditorDetectionEnv {
        path_dirs: vec![dir.clone()],
        ..ExternalEditorDetectionEnv::default()
    };

    let editors = detect_external_editors_with_env(&env);

    let zed_paths: Vec<_> = editors
        .iter()
        .filter(|editor| editor.id == "zed")
        .map(|editor| editor.path.clone())
        .collect();
    assert_eq!(zed_paths, vec![dir.join("zed.exe")]);
}

#[test]
fn detects_macos_app_bundle_fallbacks_from_configured_roots() {
    let apps = temp_dir("apps");
    let xcode = apps.join("Xcode.app");
    fs::create_dir_all(&xcode).expect("create xcode bundle");
    let env = ExternalEditorDetectionEnv {
        application_dirs: vec![apps],
        ..ExternalEditorDetectionEnv::default()
    };

    let editors = detect_external_editors_with_env(&env);

    assert!(
        editors.iter().any(|editor| {
            editor.id == "xcode" && editor.label == "Xcode" && editor.path == xcode
        })
    );
}

#[test]
fn detects_visual_studio_editions() {
    let root = temp_dir("visual-studio");
    let devenv = root.join("Microsoft Visual Studio/2022/Professional/Common7/IDE/devenv.exe");
    touch(&devenv);
    let env = ExternalEditorDetectionEnv {
        visual_studio_roots: vec![root],
        ..ExternalEditorDetectionEnv::default()
    };

    let editors = detect_external_editors_with_env(&env);

    assert!(editors.iter().any(|editor| {
        editor.id == "visual-studio-professional"
            && editor.label == "Visual Studio Professional"
            && editor.path == devenv
    }));
}

#[test]
fn terminal_vim_is_gated_by_terminal_launcher_and_shadowed_by_gui() {
    let dir = temp_dir("terminal");
    touch_executable(&dir.join(executable_name("vim")));
    let no_terminal = ExternalEditorDetectionEnv {
        path_dirs: vec![dir.clone()],
        ..ExternalEditorDetectionEnv::default()
    };
    assert!(
        detect_external_editors_with_env(&no_terminal)
            .iter()
            .all(|editor| editor.id != "vim-terminal")
    );

    touch_executable(&dir.join(executable_name("xterm")));
    let with_terminal = ExternalEditorDetectionEnv {
        path_dirs: vec![dir.clone()],
        ..ExternalEditorDetectionEnv::default()
    };
    assert!(
        detect_external_editors_with_env(&with_terminal)
            .iter()
            .any(|editor| editor.id == "vim-terminal")
    );

    touch_executable(&dir.join(executable_name("gvim")));
    let with_gui = ExternalEditorDetectionEnv {
        path_dirs: vec![dir],
        ..ExternalEditorDetectionEnv::default()
    };
    let editors = detect_external_editors_with_env(&with_gui);
    assert!(editors.iter().any(|editor| editor.id == "vim-gui"));
    assert!(editors.iter().all(|editor| editor.id != "vim-terminal"));
}

#[cfg(not(windows))]
#[test]
fn terminal_vim_launch_command_uses_terminal_specific_argv() {
    let target = Path::new("/tmp/repo");
    let editor = PathBuf::from("/usr/bin/vim");
    let setting = ExternalCodeEditorSetting::Detected {
        id: "vim-terminal".to_string(),
        path: editor.clone(),
    };

    for (terminal_name, expected_prefix) in [
        ("gnome-terminal", vec![OsString::from("--")]),
        ("xfce4-terminal", vec![OsString::from("-x")]),
        ("xterm", vec![OsString::from("-e")]),
    ] {
        let dir = temp_dir(terminal_name);
        let terminal = dir.join(terminal_name);
        touch_executable(&terminal);
        let env = ExternalEditorDetectionEnv {
            path_dirs: vec![dir],
            ..ExternalEditorDetectionEnv::default()
        };

        let command = launch_command_for_setting_with_env(&setting, target, &env)
            .expect("build terminal Vim command");

        let mut expected_args = expected_prefix;
        expected_args.push(editor.as_os_str().to_os_string());
        expected_args.push(target.as_os_str().to_os_string());
        assert_eq!(command.program, terminal);
        assert_eq!(
            command.args, expected_args,
            "expected {terminal_name} argv to pass the editor target correctly"
        );
    }
}

#[test]
fn saved_missing_selection_is_included_in_options() {
    let setting = ExternalCodeEditorSetting::Detected {
        id: "vscode".to_string(),
        path: PathBuf::from("/definitely/missing/code"),
    };

    let options = external_editor_options_from_detected(Some(&setting), Vec::new());

    assert!(options.iter().any(|option| {
            option.missing
                && option.label == "Visual Studio Code (missing)"
                && matches!(&option.kind, ExternalEditorOptionKind::Detected(saved) if saved == &setting)
        }));
}

#[test]
fn launch_command_for_detected_editor_appends_target() {
    let setting = ExternalCodeEditorSetting::Detected {
        id: "vscode".to_string(),
        path: PathBuf::from("/usr/bin/code"),
    };
    let target = Path::new("/tmp/repo");

    let command = launch_command_for_setting_with_env(
        &setting,
        target,
        &ExternalEditorDetectionEnv::default(),
    )
    .expect("build launch command");

    assert_eq!(command.program, PathBuf::from("/usr/bin/code"));
    assert_eq!(command.args, vec![OsString::from("/tmp/repo")]);
}

#[test]
fn configured_launch_command_uses_runtime_override_before_session_persist_finishes() {
    let _guard = configured_setting_override_test_guard();
    set_configured_setting_override(Some(ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/usr/bin/editor"),
        arguments: Some("--reuse-window".to_string()),
    }));

    let command = launch_command_for_configured_editor(Path::new("/tmp/repo"))
        .expect("runtime override should be launchable without reading session state");

    assert_eq!(command.program, PathBuf::from("/usr/bin/editor"));
    assert_eq!(
        command.args,
        vec![
            OsString::from("--reuse-window"),
            OsString::from("/tmp/repo")
        ]
    );
}

#[test]
fn configured_launch_command_respects_runtime_clear_before_session_persist_finishes() {
    let _guard = configured_setting_override_test_guard();
    set_configured_setting_override(None);

    let err = launch_command_for_configured_editor(Path::new("/tmp/repo")).unwrap_err();

    assert_eq!(err, ExternalEditorError::NotConfigured);
}

#[test]
fn configured_setting_override_is_thread_local_in_tests() {
    let _guard = configured_setting_override_test_guard();
    let setting = ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/usr/bin/editor"),
        arguments: None,
    };
    set_configured_setting_override(Some(setting.clone()));

    assert_eq!(
        configured_setting_preference_override(),
        Some(Some(setting))
    );

    let other_thread_override = std::thread::spawn(configured_setting_preference_override)
        .join()
        .expect("read override from another test thread");

    assert_eq!(other_thread_override, None);
}

#[test]
fn app_bundle_launch_command_routes_through_macos_open() {
    let command = macos_app_bundle_launch_command(
        Path::new("/Applications/Example.app"),
        Path::new("/tmp/repo"),
        None,
    )
    .expect("build app bundle command");

    assert_eq!(command.program, PathBuf::from("open"));
    assert_eq!(
        command.args,
        vec![
            OsString::from("-a"),
            OsString::from("/Applications/Example.app"),
            OsString::from("/tmp/repo"),
        ]
    );
}

#[test]
fn app_bundle_launch_command_routes_custom_args_through_macos_open_args() {
    let command = macos_app_bundle_launch_command(
        Path::new("/Applications/Example.app"),
        Path::new("/tmp/repo"),
        Some("--reuse-window {path}"),
    )
    .expect("build app bundle command with args");

    assert_eq!(command.program, PathBuf::from("open"));
    assert_eq!(
        command.args,
        vec![
            OsString::from("-a"),
            OsString::from("/Applications/Example.app"),
            OsString::from("--args"),
            OsString::from("--reuse-window"),
            OsString::from("/tmp/repo"),
        ]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn custom_app_bundle_setting_routes_through_macos_open() {
    let setting = ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/Applications/Example.app"),
        arguments: None,
    };

    let command = launch_command_for_setting_with_env(
        &setting,
        Path::new("/tmp/repo"),
        &ExternalEditorDetectionEnv::default(),
    )
    .expect("build custom app bundle command");

    assert_eq!(command.program, PathBuf::from("open"));
    assert_eq!(
        command.args,
        vec![
            OsString::from("-a"),
            OsString::from("/Applications/Example.app"),
            OsString::from("/tmp/repo"),
        ]
    );
}

#[test]
fn custom_launch_args_substitute_path_placeholder() {
    let setting = ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/usr/bin/editor"),
        arguments: Some("--reuse-window \"{path}\" --line 5".to_string()),
    };

    let command = launch_command_for_setting_with_env(
        &setting,
        Path::new("/tmp/project/src main.rs"),
        &ExternalEditorDetectionEnv::default(),
    )
    .expect("build custom command");

    assert_eq!(
        command.args,
        vec![
            OsString::from("--reuse-window"),
            OsString::from("/tmp/project/src main.rs"),
            OsString::from("--line"),
            OsString::from("5"),
        ]
    );
}

#[cfg(unix)]
#[test]
fn custom_launch_args_preserve_non_utf8_path_bytes_in_placeholder() {
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

    let target = PathBuf::from(OsString::from_vec(b"/tmp/gitcomet-\xff/repo".to_vec()));
    let setting = ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/usr/bin/editor"),
        arguments: Some("--open {path} --marker=before-{path}-after".to_string()),
    };

    let command = launch_command_for_setting_with_env(
        &setting,
        &target,
        &ExternalEditorDetectionEnv::default(),
    )
    .expect("build custom command");

    assert_eq!(command.args[0], OsString::from("--open"));
    assert_eq!(
        command.args[1].as_os_str().as_bytes(),
        target.as_os_str().as_bytes()
    );
    assert_eq!(
        command.args[2].as_os_str().as_bytes(),
        b"--marker=before-/tmp/gitcomet-\xff/repo-after"
    );
}

#[test]
fn custom_launch_args_append_target_when_placeholder_is_omitted() {
    let setting = ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/usr/bin/editor"),
        arguments: Some("--reuse-window".to_string()),
    };

    let command = launch_command_for_setting_with_env(
        &setting,
        Path::new("/tmp/repo"),
        &ExternalEditorDetectionEnv::default(),
    )
    .expect("build custom command");

    assert_eq!(
        command.args,
        vec![
            OsString::from("--reuse-window"),
            OsString::from("/tmp/repo")
        ]
    );
}

#[test]
fn custom_launch_args_preserve_windows_style_backslashes() {
    let setting = ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("editor.exe"),
        arguments: Some("--profile \"C:\\Users\\Sampo Main\\Editor\"".to_string()),
    };

    let command = launch_command_for_setting_with_env(
        &setting,
        Path::new("C:\\repo"),
        &ExternalEditorDetectionEnv::default(),
    )
    .expect("build custom command");

    assert_eq!(
        command.args,
        vec![
            OsString::from("--profile"),
            OsString::from("C:\\Users\\Sampo Main\\Editor"),
            OsString::from("C:\\repo"),
        ]
    );
}

#[test]
fn custom_launch_args_preserve_trailing_windows_backslash_before_closing_quote() {
    let setting = ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("editor.exe"),
        arguments: Some("--profile \"C:\\Tools\\Editor\\\"".to_string()),
    };

    let command = launch_command_for_setting_with_env(
        &setting,
        Path::new("C:\\repo"),
        &ExternalEditorDetectionEnv::default(),
    )
    .expect("build custom command");

    assert_eq!(
        command.args,
        vec![
            OsString::from("--profile"),
            OsString::from("C:\\Tools\\Editor\\"),
            OsString::from("C:\\repo"),
        ]
    );
}

#[test]
fn custom_launch_args_reject_unclosed_quotes() {
    let setting = ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/usr/bin/editor"),
        arguments: Some("\"--bad".to_string()),
    };

    let err = launch_command_for_setting_with_env(
        &setting,
        Path::new("/tmp/repo"),
        &ExternalEditorDetectionEnv::default(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ExternalEditorError::InvalidCustomArguments(_)
    ));
}

#[test]
fn configured_setting_caches_session_value_on_repeated_calls() {
    let _guard = configured_setting_override_test_guard();

    assert!(session_setting_cache_value().is_none());

    let first = configured_setting();
    let cached = session_setting_cache_value();

    assert_eq!(
        first,
        cached.and_then(|c| c),
        "configured_setting() should populate the cache with the session-derived value"
    );

    let second = configured_setting();
    assert_eq!(
        first, second,
        "repeated calls should return the same cached value"
    );
}

#[test]
fn configured_setting_override_invalidates_session_cache() {
    let _guard = configured_setting_override_test_guard();

    configured_setting();
    assert!(
        session_setting_cache_value().is_some(),
        "cache should be populated after first configured_setting() call"
    );

    set_configured_setting_override(Some(ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/usr/bin/editor"),
        arguments: None,
    }));

    assert!(
        session_setting_cache_value().is_none(),
        "cache should be invalidated after set_configured_setting_override is called"
    );

    let result = configured_setting();
    assert!(
        result.is_some(),
        "should return override value without touching cache"
    );
}

#[test]
fn configured_setting_override_clear_invalidates_session_cache() {
    let _guard = configured_setting_override_test_guard();

    set_configured_setting_override(Some(ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/usr/bin/editor"),
        arguments: None,
    }));
    assert!(configured_setting().is_some(), "override should be active");

    clear_configured_setting_override();

    assert!(
        session_setting_cache_value().is_none(),
        "cache should be invalidated when override is cleared"
    );

    assert_eq!(
        configured_setting_preference_override(),
        None,
        "override should be cleared"
    );
}
