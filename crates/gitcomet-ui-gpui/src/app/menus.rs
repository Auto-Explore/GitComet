//! Application menus: the macOS menu bar and recent repositories.

use super::*;

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema, Action)]
#[action(namespace = app_menu)]
#[serde(deny_unknown_fields)]
pub(super) struct OpenRecentRepository {
    pub(super) storage_key: String,
}

#[cfg(target_os = "macos")]
pub(super) fn install_macos_app_menu(cx: &mut App, backend: Arc<dyn GitBackend>) {
    let recent_repo_backend = Arc::clone(&backend);
    cx.on_action(move |recent: &OpenRecentRepository, cx| {
        let path = session::path_from_storage_key(&recent.storage_key);
        let backend = Arc::clone(&recent_repo_backend);
        cx.defer(move |cx| {
            open_repository_in_existing_or_new_window(cx, backend, path);
        });
    });

    cx.on_action(|_: &ApplyPatch, cx| {
        cx.defer(prompt_apply_patch);
    });

    cx.on_action(|_: &CheckForUpdates, cx| {
        let _ = check_for_updates_in_active_or_existing_normal_window(cx);
    });

    let clone_backend = Arc::clone(&backend);
    cx.on_action(move |_: &CloneRepository, cx| {
        let backend = Arc::clone(&clone_backend);
        cx.defer(move |cx| open_clone_repository_in_existing_or_new_window(cx, backend));
    });

    let initialize_backend = Arc::clone(&backend);
    cx.on_action(move |_: &InitializeRepository, cx| {
        let backend = Arc::clone(&initialize_backend);
        cx.defer(move |cx| prompt_initialize_repository_in_existing_or_new_window(cx, backend));
    });

    refresh_macos_app_menus(cx);
}

#[cfg(target_os = "macos")]
pub(super) fn macos_app_menus(cx: &mut App) -> Vec<Menu> {
    let mut menus = macos_app_menus_with_options(
        crate::external_editor::configured_setting().is_some(),
        find_normal_gitcomet_window(cx).is_some(),
    );
    let extension_items = crate::view::extension_host::macos_menu_items(cx);
    if !extension_items.is_empty()
        && let Some(file) = menus.get_mut(1)
    {
        // Above the Close group (separator, Close, Close Window), which stays last.
        let at = file.items.len().saturating_sub(3);
        file.items.splice(
            at..at,
            std::iter::once(MenuItem::separator()).chain(extension_items),
        );
    }
    menus
}

/// The menus as they look with a normal window open, so the tests that care
/// about the external-editor item do not have to restate that half.
#[cfg(all(test, target_os = "macos"))]
pub(super) fn macos_app_menus_with_external_editor(external_editor_configured: bool) -> Vec<Menu> {
    macos_app_menus_with_options(external_editor_configured, true)
}

#[cfg(target_os = "macos")]
pub(super) fn macos_app_menus_with_options(
    external_editor_configured: bool,
    normal_window_available: bool,
) -> Vec<Menu> {
    let mut file_items = vec![
        MenuItem::action("New Window", NewWindow),
        MenuItem::separator(),
        MenuItem::action(crate::menu_labels::OPEN_REPOSITORY, OpenRepository),
        MenuItem::action(crate::menu_labels::CLONE_REPOSITORY, CloneRepository),
        MenuItem::action(
            crate::menu_labels::INITIALIZE_REPOSITORY,
            InitializeRepository,
        ),
        MenuItem::action("Switch Repository…", SwitchRepository),
        MenuItem::action("Open Workspace…", OpenWorkspace),
    ];

    let recent_repo_items = recent_repo_menu_items();
    if !recent_repo_items.is_empty() {
        file_items.push(MenuItem::submenu(Menu {
            name: "Recent Repositories".into(),
            items: recent_repo_items,
            disabled: false,
        }));
    }
    file_items.push(MenuItem::separator());
    if external_editor_configured {
        file_items.push(MenuItem::action(
            crate::menu_labels::OPEN_IN_CODE_EDITOR,
            OpenInCodeEditor,
        ));
    }

    file_items.extend([
        MenuItem::action(
            crate::menu_labels::OPEN_IN_FILE_EXPLORER,
            LocateFileInExplorer,
        ),
        MenuItem::action(
            crate::menu_labels::OPEN_REMOTE_IN_BROWSER,
            OpenRemoteInBrowser,
        ),
        MenuItem::action(crate::menu_labels::APPLY_PATCH, ApplyPatch),
        MenuItem::action(crate::menu_labels::CHECK_FOR_UPDATES, CheckForUpdates).disabled(
            manual_update_check_menu_disabled(
                crate::view::update_checks_disabled_by_environment()
                    || !crate::view::update_checks_available(),
                normal_window_available,
            ),
        ),
        MenuItem::separator(),
        MenuItem::action("Close", Close),
        MenuItem::action("Close Window", CloseWindow),
    ]);

    let name = identity::current().display_name();
    vec![
        Menu {
            name: name.to_string().into(),
            items: vec![
                MenuItem::action(crate::menu_labels::COMMAND_PALETTE, ToggleCommandPalette),
                MenuItem::action(crate::menu_labels::SETTINGS, OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(format!("Hide {name}"), Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action(format!("Quit {name}"), Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: file_items,
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", crate::kit::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", crate::kit::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", crate::kit::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", crate::kit::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", crate::kit::Paste, OsAction::Paste),
                MenuItem::separator(),
                MenuItem::os_action("Select All", crate::kit::SelectAll, OsAction::SelectAll),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![MenuItem::action("Reflog", ShowReflog)],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", MinimizeWindow),
                MenuItem::action("Zoom", ZoomWindow),
                MenuItem::separator(),
                MenuItem::action("Zoom In", IncreaseUiScale),
                MenuItem::action("Zoom Out", DecreaseUiScale),
                MenuItem::action("Reset Zoom", ResetUiScale),
                MenuItem::separator(),
                MenuItem::action("Previous Repository", PreviousRepository),
                MenuItem::action("Next Repository", NextRepository),
                MenuItem::separator(),
                MenuItem::action("Toggle Full Screen", ToggleFullScreen),
            ],
            disabled: false,
        },
    ]
}

#[cfg(target_os = "macos")]
pub(crate) fn refresh_macos_app_menus(cx: &mut App) {
    let menus = macos_app_menus(cx);
    cx.set_menus(menus);
}

#[cfg(target_os = "macos")]
pub(super) fn refresh_macos_app_menus_for_external_editor(cx: &mut App, configured: bool) {
    let normal_window_available = find_normal_gitcomet_window(cx).is_some();
    cx.set_menus(macos_app_menus_with_options(
        configured,
        normal_window_available,
    ));
}

#[cfg(target_os = "macos")]
pub(super) fn recent_repo_menu_items() -> Vec<MenuItem> {
    session::load()
        .recent_repos
        .into_iter()
        .map(|path| {
            MenuItem::action(
                recent_repository_label(&path),
                OpenRecentRepository {
                    storage_key: session::path_storage_key(&path),
                },
            )
        })
        .collect()
}

/// Labels the macOS "Recent Repositories" menu items. Other platforms only
/// reach the recents through the repository switcher, which builds its own rows.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn recent_repository_label(path: &Path) -> String {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.display().to_string();
    };
    let Some(parent) = path.parent() else {
        return name.to_string();
    };
    format!("{name} - {}", parent.display())
}
