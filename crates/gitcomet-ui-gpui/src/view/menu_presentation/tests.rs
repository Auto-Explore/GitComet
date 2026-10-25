use super::*;

#[test]
fn host_menu_presentation_preserves_assets_tones_and_empty_icon_slots() {
    let path = "extensions/com.example.review/icons/review.svg";
    assert_eq!(
        entry_presentation("Remove review", Some(path.into()), false),
        (
            ContextMenuIconSlot::Icon(path.into()),
            ContextMenuTone::Danger
        ),
    );
    assert_eq!(
        entry_presentation("Push review", None, false),
        (ContextMenuIconSlot::None, ContextMenuTone::Success),
    );
    assert_eq!(
        entry_presentation("Copy review", None, true),
        (ContextMenuIconSlot::Reserved, ContextMenuTone::Accent),
    );
    assert_eq!(
        entry_presentation("Pull review", Some("↓".into()), false),
        (
            ContextMenuIconSlot::Icon("icons/arrow_down.svg".into()),
            ContextMenuTone::Warning,
        ),
    );
}

#[test]
fn context_menu_icon_path_trims_and_keeps_paths_without_checking_assets() {
    // Whether the asset exists is `every_icon_path_literal_in_the_crate_loads`'s job.
    for (icon, path) in [
        ("  icons/trash.svg ", "icons/trash.svg"),
        ("icons/not_an_asset.svg", "icons/not_an_asset.svg"),
    ] {
        assert_eq!(context_menu_icon_path(icon, "Copy"), Some(path), "{icon}");
    }
}
#[test]
fn context_menu_icon_path_maps_named_link_icons() {
    assert_eq!(
        context_menu_icon_path("link", "test"),
        Some("icons/link.svg")
    );
    assert_eq!(
        context_menu_icon_path("unlink", "test"),
        Some("icons/unlink.svg")
    );
}
#[test]
fn explicit_menu_assets_take_precedence_over_english_label_inference() {
    for path in [
        "extensions/com.example.review/icons/review.svg",
        "icons/terminal.svg",
    ] {
        for label in ["Remove review", "Pull review", "Copy review", "Open review"] {
            assert_eq!(context_menu_icon_path(path, label), Some(path));
        }
    }
}
#[test]
fn context_menu_icon_path_uses_label_fallbacks() {
    assert_eq!(
        context_menu_icon_path("", "Pull (merge)"),
        Some("icons/arrow_down.svg")
    );
    assert_eq!(
        context_menu_icon_path("", "Remove remote"),
        Some("icons/trash.svg")
    );
    assert_eq!(
        context_menu_icon_path("", "Squash into current"),
        Some("icons/arrow_right.svg")
    );
}
#[test]
fn context_menu_icon_color_preserves_destructive_and_warning_semantics() {
    assert_eq!(
        context_menu_tone("Delete branch", Some("icons/trash.svg")),
        ContextMenuTone::Danger
    );
    assert_eq!(
        context_menu_tone("Close", Some("icons/repo_tab_close.svg")),
        ContextMenuTone::Accent
    );
    assert_eq!(
        context_menu_tone("Force push (with lease)…", Some("icons/warning.svg")),
        ContextMenuTone::Danger
    );
    assert_eq!(
        context_menu_tone("Discard changes", Some("icons/warning.svg")),
        ContextMenuTone::Warning
    );
    // Creating a worktree reads like the green "Create branch" beside it;
    // opening an existing one stays neutral.
    assert_eq!(
        context_menu_tone("Open in new worktree", Some("icons/git_worktree.svg")),
        ContextMenuTone::Success
    );
    assert_eq!(
        context_menu_tone("Open worktree", Some("icons/git_worktree.svg")),
        ContextMenuTone::Accent
    );
    // The explorer's reversible Trash stays neutral; permanent deletion is red.
    assert_eq!(
        context_menu_tone("Trash", Some("icons/trash.svg")),
        ContextMenuTone::Accent
    );
    assert_eq!(
        context_menu_tone("Delete permanently…", Some("icons/trash.svg")),
        ContextMenuTone::Danger
    );
    assert_eq!(
        context_menu_tone("Cut", Some("icons/scissors.svg")),
        ContextMenuTone::Accent
    );
    assert_eq!(
        context_menu_tone("Add to .gitignore", Some("icons/eye_off.svg")),
        ContextMenuTone::Accent
    );
}
