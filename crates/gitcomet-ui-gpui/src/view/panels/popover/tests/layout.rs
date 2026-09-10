use super::*;

#[test]
fn popover_width_spec_scales_with_zoom() {
    let spec = popover_width_spec(&PopoverKind::RepoPicker).expect("repo picker width");
    let default_scale = ui_scale::UiScale::from_percent(100);
    let zoomed_scale = ui_scale::UiScale::from_percent(200);

    assert_eq!(spec.preferred_px(default_scale), px(420.0));
    assert_eq!(spec.preferred_px(zoomed_scale), px(840.0));
    assert_eq!(spec.max_px(zoomed_scale), px(1640.0));
}

#[test]
fn mergetool_settings_menu_is_wider_than_diff_actions() {
    let scale = ui_scale::UiScale::from_percent(100);
    let mergetool =
        popover_width_spec(&PopoverKind::MergetoolSettingsMenu).expect("mergetool menu width");
    let diff_actions =
        popover_width_spec(&PopoverKind::DiffActionMenu).expect("diff actions menu width");

    assert_eq!(mergetool.preferred_px(scale), px(320.0));
    assert!(mergetool.preferred_px(scale) > diff_actions.preferred_px(scale));
    assert!(mergetool.min_px(scale) > diff_actions.min_px(scale));
}

/// The sort menu's labels name both the key and the direction ("File type:
/// Descending"), which does not fit the narrow bucket it used to share with the
/// icon-and-a-word menus -- it ellipsised the longest option.
#[test]
fn sort_menu_is_wider_than_the_narrow_menus_it_used_to_share() {
    let scale = ui_scale::UiScale::from_percent(100);
    let sort = popover_width_spec(&PopoverKind::CommitFileSortMenu {
        list: crate::view::rows::FileListId::CommitFiles,
    })
    .expect("sort menu width");
    let narrow =
        popover_width_spec(&PopoverKind::UiScalePicker).expect("a still-narrow menu for contrast");

    assert!(sort.preferred_px(scale) > narrow.preferred_px(scale));
    assert!(sort.max_px(scale) > narrow.max_px(scale));

    // Padding, the check-mark column and its gap, and the trailing gap the row
    // spends before its (empty) end slot -- see `components::context_menu`.
    const ROW_CHROME_PX: f32 = 8.0 + 16.0 + 8.0 + 8.0 + 20.0;
    // The header-row budget's per-character estimate, reused here.
    const CHAR_PX: f32 = 5.2;
    let longest = crate::view::rows::CommitFileSort::ALL
        .into_iter()
        .map(|sort| sort.label().chars().count())
        .max()
        .expect("at least one sort");
    assert!(
        sort.min_px(scale) >= px(ROW_CHROME_PX + CHAR_PX * longest as f32),
        "the longest label must fit without ellipsis at the menu's narrowest",
    );
}

#[test]
fn repository_tab_menu_has_dedicated_wider_layout() {
    let scale = ui_scale::UiScale::from_percent(100);
    let repo_tab = popover_width_spec(&PopoverKind::RepoTabMenu { repo_id: RepoId(1) })
        .expect("repository tab menu width");

    assert_eq!(repo_tab.preferred_px(scale), px(360.0));
    assert_eq!(repo_tab.min_px(scale), px(360.0));
    assert!(repo_tab.preferred_px(scale) > DEFAULT_CONTEXT_MENU_WIDTH.preferred_px(scale));
}

#[test]
fn application_menu_is_wide_enough_for_editor_action_and_shortcut() {
    let scale = ui_scale::UiScale::from_percent(100);
    let app_menu = popover_width_spec(&PopoverKind::AppMenu).expect("application menu width");

    assert_eq!(app_menu.preferred_px(scale), px(320.0));
    assert_eq!(app_menu.min_px(scale), px(320.0));
    assert!(app_menu.preferred_px(scale) > DEFAULT_CONTEXT_MENU_WIDTH.preferred_px(scale));
}

#[test]
fn branch_exists_prompt_uses_the_wide_dialog_layout() {
    let scale = ui_scale::UiScale::from_percent(100);
    let dialog = popover_width_spec(&PopoverKind::BranchExistsPrompt {
        repo_id: RepoId(1),
        name: "feature".to_string(),
        target: "origin/a-long-feature-branch".to_string(),
        operation: BranchExistsPromptOperation::CreateBranch,
    })
    .expect("branch-exists dialog width");

    assert_eq!(dialog.preferred_px(scale), px(540.0));
    assert_eq!(dialog.min_px(scale), px(540.0));
}

#[test]
fn choose_popover_anchor_corner_prefers_side_with_more_space() {
    assert_eq!(
        choose_popover_anchor_corner(Anchor::TopRight, px(260.0), px(640.0), px(420.0),),
        Anchor::TopLeft,
    );
    assert_eq!(
        choose_popover_anchor_corner(Anchor::BottomLeft, px(500.0), px(260.0), px(420.0),),
        Anchor::BottomRight,
    );
}

#[gpui::test]
fn reword_dialog_with_long_squash_message_stays_within_viewport(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let description = (0..120)
        .map(|ix| format!("Squashed commit message {ix}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let original_message = format!("Combined subject\n\n{description}");

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_centered(
                    PopoverKind::RebaseReword {
                        ix: 0,
                        original_action: InteractiveRebaseAction::Pick,
                        original_message,
                    },
                    window,
                    cx,
                );
            });
        });
    });
    crate::view::test_support::redraw(cx);

    let popover_bounds = cx
        .debug_bounds("app_popover")
        .expect("expected reword dialog to render");
    let mut viewport_height = px(0.0);
    cx.update(|window, _app| {
        viewport_height = window.window_bounds().get_bounds().size.height;
    });

    assert!(
        popover_bounds.bottom() <= viewport_height,
        "reword dialog bottom {:?} exceeded viewport height {:?}",
        popover_bounds.bottom(),
        viewport_height,
    );
}
