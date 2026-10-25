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
fn context_menu_icon_path_accepts_direct_svg_paths() {
    let paths = [
        "icons/link.svg",
        "icons/unlink.svg",
        "icons/plus.svg",
        "icons/minus.svg",
        "icons/question.svg",
        "icons/warning.svg",
        "icons/check.svg",
        "icons/git_branch.svg",
        "icons/arrow_down.svg",
        "icons/arrow_up.svg",
        "icons/arrow_down_to_line.svg",
        "icons/arrow_up_to_line.svg",
        "icons/chevron_down.svg",
        "icons/chevron_right.svg",
        "icons/broom.svg",
        "icons/stash.svg",
        "icons/tag.svg",
        "icons/trash.svg",
        "icons/repo_tab_close.svg",
        "icons/refresh.svg",
        "icons/open_external.svg",
        "icons/file.svg",
        "icons/folder.svg",
        "icons/copy.svg",
        "icons/box.svg",
        "icons/menu.svg",
        "icons/list_tree.svg",
        "icons/list_groups.svg",
        "icons/swap.svg",
        "icons/squash_arrow.svg",
        "icons/arrow_right.svg",
        "icons/infinity.svg",
        "icons/arrow_left.svg",
        "icons/undo.svg",
        "icons/pencil.svg",
        "icons/cloud.svg",
        "icons/computer.svg",
        "icons/pin.svg",
        "icons/cog.svg",
        "icons/disk.svg",
        "icons/generic_close.svg",
        "icons/git_commit.svg",
        "icons/file_plus.svg",
        "icons/folder_plus.svg",
        "icons/scissors.svg",
        "icons/clipboard_paste.svg",
        "icons/copy_plus.svg",
        "icons/redo.svg",
        "icons/eye_off.svg",
    ];

    for path in paths {
        assert_eq!(context_menu_icon_path(path, "test"), Some(path));
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
#[test]
fn context_menu_icon_path_covers_all_context_menu_svg_icons() {
    let paths = [
        "icons/plus.svg",
        "icons/check.svg",
        "icons/git_branch.svg",
        "icons/arrow_down.svg",
        "icons/arrow_up.svg",
        "icons/arrow_down_to_line.svg",
        "icons/arrow_up_to_line.svg",
        "icons/chevron_down.svg",
        "icons/chevron_right.svg",
        "icons/broom.svg",
        "icons/stash.svg",
        "icons/tag.svg",
        "icons/trash.svg",
        "icons/repo_tab_close.svg",
        "icons/refresh.svg",
        "icons/open_external.svg",
        "icons/file.svg",
        "icons/folder.svg",
        "icons/copy.svg",
        "icons/box.svg",
        "icons/infinity.svg",
        "icons/swap.svg",
        "icons/squash_arrow.svg",
        "icons/arrow_right.svg",
        "icons/arrow_left.svg",
        "icons/pencil.svg",
        "icons/link.svg",
        "icons/unlink.svg",
        "icons/warning.svg",
        "icons/minus.svg",
        "icons/cloud.svg",
        "icons/computer.svg",
        "icons/pin.svg",
        "icons/cog.svg",
        "icons/disk.svg",
        "icons/generic_close.svg",
        "icons/git_commit.svg",
        "icons/file_plus.svg",
        "icons/folder_plus.svg",
        "icons/scissors.svg",
        "icons/clipboard_paste.svg",
        "icons/copy_plus.svg",
        "icons/redo.svg",
        "icons/eye_off.svg",
    ];
    for path in paths {
        assert_eq!(
            context_menu_icon_path(path, "test"),
            Some(path),
            "missing direct SVG support for context-menu icon path: {path}"
        );
    }
}
