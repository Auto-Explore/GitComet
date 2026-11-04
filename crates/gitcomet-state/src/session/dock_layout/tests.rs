use super::*;

fn p(id: &str) -> PanelId {
    PanelId::from(id)
}

fn s(id: &str) -> SectionId {
    SectionId::from(id)
}

fn ids(sections: &[SectionId]) -> Vec<&str> {
    sections.iter().map(SectionId::as_str).collect()
}

#[test]
fn factory_layout_matches_the_window_before_layouts_were_configurable() {
    let layout = DockLayout::factory();
    assert_eq!(
        layout.panels_in(DockSide::Left, DockHalf::Primary),
        vec![
            p(panel_ids::BRANCHES),
            p(panel_ids::FILES),
            p(panel_ids::FILE_HISTORY)
        ]
    );
    assert_eq!(
        layout.panels_in(DockSide::Right, DockHalf::Primary),
        vec![p(panel_ids::CHANGES)]
    );
    assert_eq!(
        layout.panels_in(DockSide::Bottom, DockHalf::Primary),
        vec![p(panel_ids::TERMINAL), p(panel_ids::REFLOG)]
    );
    assert_eq!(
        ids(layout.sections_in(&p(panel_ids::BRANCHES))),
        DEFAULT_BRANCHES_SECTIONS.to_vec()
    );
    assert_eq!(
        ids(layout.sections_in(&p(panel_ids::CHANGES))),
        DEFAULT_CHANGES_SECTIONS.to_vec()
    );
    assert_eq!(
        layout.section_location(&s(section_ids::TAGS)),
        SectionLocation::Hidden
    );
    assert!(layout.is_panel_visible(&p(panel_ids::BRANCHES)));
    assert!(layout.is_panel_visible(&p(panel_ids::CHANGES)));
    assert!(!layout.is_panel_visible(&p(panel_ids::FILES)));
    assert!(!layout.is_dock_visible(DockSide::Bottom));
}

#[test]
fn legacy_sizes_and_collapse_migrate_into_the_factory_layout() {
    let legacy = WorkspaceLayout {
        sidebar_width: Some(333),
        details_width: Some(410),
        sidebar_collapsed: true,
        change_tracking_height: Some(220),
        untracked_height: Some(90),
        dock: None,
    };
    let layout = DockLayout::from_legacy(&legacy);
    assert_eq!(layout.dock(DockSide::Left).size, Some(333));
    assert!(layout.dock(DockSide::Left).hidden);
    assert_eq!(layout.dock(DockSide::Right).size, Some(410));
    assert_eq!(
        layout.section_weights.get(&s(section_ids::UNSTAGED)),
        Some(&220)
    );
    assert_eq!(
        layout.section_weights.get(&s(section_ids::UNTRACKED)),
        Some(&90)
    );
    assert!(!layout.is_panel_visible(&p(panel_ids::BRANCHES)));
}

#[test]
fn moving_a_visible_panel_keeps_it_visible_and_its_old_half_falls_back() {
    let mut layout = DockLayout::factory();
    assert!(layout.move_panel(
        &p(panel_ids::BRANCHES),
        DockSide::Right,
        DockHalf::Primary,
        None
    ));
    assert!(layout.is_panel_visible(&p(panel_ids::BRANCHES)));
    assert!(!layout.is_panel_visible(&p(panel_ids::CHANGES)));
    assert_eq!(
        layout.shown_panel(DockSide::Left, DockHalf::Primary),
        Some(p(panel_ids::FILES))
    );
    assert_eq!(
        layout.panels_in(DockSide::Right, DockHalf::Primary),
        vec![p(panel_ids::CHANGES), p(panel_ids::BRANCHES)]
    );
}

#[test]
fn moving_before_a_tab_reorders_within_a_half() {
    let mut layout = DockLayout::factory();
    layout.move_panel(
        &p(panel_ids::FILE_HISTORY),
        DockSide::Left,
        DockHalf::Primary,
        Some(&p(panel_ids::BRANCHES)),
    );
    assert_eq!(
        layout.panels_in(DockSide::Left, DockHalf::Primary),
        vec![
            p(panel_ids::FILE_HISTORY),
            p(panel_ids::BRANCHES),
            p(panel_ids::FILES)
        ]
    );
}

#[test]
fn split_half_shows_a_second_panel_and_closing_it_keeps_the_dock() {
    let mut layout = DockLayout::factory();
    layout.move_panel(&p(panel_ids::FILES), DockSide::Left, DockHalf::Split, None);
    layout.show_panel(&p(panel_ids::FILES));
    assert!(layout.is_panel_visible(&p(panel_ids::BRANCHES)));
    assert!(layout.is_panel_visible(&p(panel_ids::FILES)));
    assert!(layout.hide_panel(&p(panel_ids::FILES)));
    assert!(layout.is_panel_visible(&p(panel_ids::BRANCHES)));
    assert!(!layout.is_panel_visible(&p(panel_ids::FILES)));
}

#[test]
fn toggling_the_primary_panel_collapses_its_dock_like_the_old_sidebar_toggle() {
    let mut layout = DockLayout::factory();
    assert!(layout.toggle_panel(&p(panel_ids::BRANCHES)));
    assert!(layout.dock(DockSide::Left).hidden);
    assert!(layout.toggle_panel(&p(panel_ids::BRANCHES)));
    assert!(layout.is_panel_visible(&p(panel_ids::BRANCHES)));
}

#[test]
fn hide_all_stashes_open_docks_and_a_second_call_restores_them() {
    let mut layout = DockLayout::factory();
    layout.show_panel(&p(panel_ids::TERMINAL));
    assert!(layout.toggle_hide_all());
    assert!(
        DockSide::ALL
            .iter()
            .all(|side| !layout.is_dock_visible(*side))
    );
    assert!(layout.toggle_hide_all());
    assert!(
        DockSide::ALL
            .iter()
            .all(|side| layout.is_dock_visible(*side))
    );
    // Nothing visible and nothing stashed: no change.
    for side in DockSide::ALL {
        layout.set_dock_hidden(side, true);
    }
    layout.hide_all_stash = None;
    assert!(!layout.toggle_hide_all());
}

#[test]
fn sizes_and_split_ratios_are_clamped() {
    let mut layout = DockLayout::factory();
    layout.set_dock_size(DockSide::Left, Some(1));
    assert_eq!(layout.dock(DockSide::Left).size, Some(MIN_DOCK_SIZE));
    layout.set_dock_size(DockSide::Left, Some(1_000_000));
    assert_eq!(layout.dock(DockSide::Left).size, Some(MAX_DOCK_SIZE));
    layout.set_dock_split(DockSide::Left, 5);
    assert_eq!(
        layout.dock(DockSide::Left).split_permille,
        MIN_SPLIT_PERMILLE
    );
    layout.set_dock_split(DockSide::Left, 999);
    assert_eq!(
        layout.dock(DockSide::Left).split_permille,
        MAX_SPLIT_PERMILLE
    );
}

#[test]
fn sections_move_between_panels_and_keep_one_home() {
    let mut layout = DockLayout::factory();
    assert!(layout.move_section(
        &s(section_ids::STAGED),
        &p(panel_ids::BRANCHES),
        Some(&s(section_ids::LOCAL_BRANCHES))
    ));
    assert_eq!(
        layout.sections_in(&p(panel_ids::BRANCHES)).first(),
        Some(&s(section_ids::STAGED))
    );
    assert!(
        !layout
            .sections_in(&p(panel_ids::CHANGES))
            .contains(&s(section_ids::STAGED))
    );
    // Terminal cannot hold sections.
    assert!(!layout.move_section(&s(section_ids::STASHES), &p(panel_ids::TERMINAL), None));
}

#[test]
fn hidden_sections_come_back_in_their_factory_position() {
    let mut layout = DockLayout::factory();
    assert!(layout.hide_section(&s(section_ids::WORKTREES)));
    assert_eq!(
        layout.section_location(&s(section_ids::WORKTREES)),
        SectionLocation::Hidden
    );
    assert!(layout.show_section(&s(section_ids::WORKTREES), &p(panel_ids::BRANCHES)));
    assert_eq!(
        ids(layout.sections_in(&p(panel_ids::BRANCHES))),
        DEFAULT_BRANCHES_SECTIONS.to_vec()
    );
    // Tags goes after the factory sections it follows in the catalogue.
    layout.show_section(&s(section_ids::TAGS), &p(panel_ids::BRANCHES));
    assert_eq!(
        layout.sections_in(&p(panel_ids::BRANCHES)).last(),
        Some(&s(section_ids::TAGS))
    );
}

#[test]
fn custom_panels_take_sections_and_return_them_when_removed() {
    let mut layout = DockLayout::factory();
    let panel = layout.add_custom_panel("Review", DockSide::Left);
    assert!(panel.is_custom());
    assert!(layout.is_panel_visible(&panel));
    layout.move_section(&s(section_ids::STASHES), &panel, None);
    layout.move_section(&s(section_ids::STAGED), &panel, None);
    assert_eq!(
        ids(layout.sections_in(&panel)),
        vec![section_ids::STASHES, section_ids::STAGED]
    );
    assert!(layout.rename_custom_panel(&panel, "Daily"));
    assert_eq!(layout.custom_panel_title(&panel), Some("Daily"));
    assert!(layout.remove_custom_panel(&panel));
    assert!(layout.placement(&panel).is_none());
    assert_eq!(
        ids(layout.sections_in(&p(panel_ids::BRANCHES))),
        DEFAULT_BRANCHES_SECTIONS.to_vec()
    );
    assert_eq!(
        ids(layout.sections_in(&p(panel_ids::CHANGES))),
        DEFAULT_CHANGES_SECTIONS.to_vec()
    );
    // Ids are not reused while another custom panel exists.
    let first = layout.add_custom_panel("", DockSide::Right);
    let second = layout.add_custom_panel("", DockSide::Right);
    assert_ne!(first, second);
    assert_eq!(layout.custom_panel_title(&first), Some("Panel 1"));
}

#[test]
fn restoring_the_branches_panel_rebuilds_todays_left_hand_menu() {
    let mut layout = DockLayout::factory();
    let custom = layout.add_custom_panel("Mine", DockSide::Right);
    for section in DEFAULT_BRANCHES_SECTIONS.iter().rev() {
        layout.move_section(&s(section), &custom, None);
    }
    layout.show_section(&s(section_ids::TAGS), &custom);
    layout.move_panel(
        &p(panel_ids::BRANCHES),
        DockSide::Bottom,
        DockHalf::Split,
        None,
    );
    layout.set_dock_hidden(DockSide::Left, true);

    assert!(layout.restore_panel(&p(panel_ids::BRANCHES)));
    assert_eq!(
        layout.placement(&p(panel_ids::BRANCHES)),
        Some(&PanelPlacement::new(DockSide::Left, DockHalf::Primary, 10))
    );
    assert_eq!(
        ids(layout.sections_in(&p(panel_ids::BRANCHES))),
        DEFAULT_BRANCHES_SECTIONS.to_vec()
    );
    assert!(layout.is_panel_visible(&p(panel_ids::BRANCHES)));
    // Tags, which the user added elsewhere, stays where the user put it.
    assert_eq!(ids(layout.sections_in(&custom)), vec![section_ids::TAGS]);
}

#[test]
fn restore_default_keeps_sizes_and_drops_custom_panels() {
    let mut layout = DockLayout::factory();
    layout.set_dock_size(DockSide::Left, Some(300));
    let custom = layout.add_custom_panel("x", DockSide::Left);
    layout.move_section(&s(section_ids::STAGED), &custom, None);
    assert!(layout.restore_default());
    assert_eq!(layout.dock(DockSide::Left).size, Some(300));
    assert!(layout.custom_panels.is_empty());
    assert_eq!(
        ids(layout.sections_in(&p(panel_ids::CHANGES))),
        DEFAULT_CHANGES_SECTIONS.to_vec()
    );
}

#[test]
fn reconcile_adds_late_panels_and_sections_but_respects_hidden_ones() {
    let mut layout = DockLayout::factory();
    let ext_panel = p("ext:lint/panel");
    let ext_section = s("ext:lint/section");
    layout.hide_section(&s(section_ids::STASHES));
    assert!(layout.reconcile(
        &[(
            ext_panel.clone(),
            PanelPlacement::new(DockSide::Bottom, DockHalf::Primary, 0)
        )],
        &[
            (ext_section.clone(), p(panel_ids::BRANCHES)),
            (s(section_ids::STASHES), p(panel_ids::BRANCHES)),
        ],
    ));
    assert_eq!(
        layout.panels_in(DockSide::Bottom, DockHalf::Primary).last(),
        Some(&ext_panel)
    );
    assert_eq!(
        layout.section_panel(&ext_section),
        Some(p(panel_ids::BRANCHES))
    );
    assert_eq!(
        layout.section_location(&s(section_ids::STASHES)),
        SectionLocation::Hidden
    );
    // A second reconcile is a no-op.
    assert!(!layout.reconcile(
        &[(
            ext_panel,
            PanelPlacement::new(DockSide::Bottom, DockHalf::Primary, 0)
        )],
        &[(ext_section, p(panel_ids::BRANCHES))],
    ));
}

#[test]
fn sanitize_repairs_duplicates_orphans_and_missing_builtins() {
    let mut layout = DockLayout::factory();
    layout.contents.insert(
        p(panel_ids::CHANGES),
        vec![s(section_ids::STAGED), s(section_ids::STAGED)],
    );
    layout
        .contents
        .insert(p(panel_ids::TERMINAL), vec![s(section_ids::UNSTAGED)]);
    layout.panels.remove(&p(panel_ids::REFLOG));
    layout.panels.insert(
        p("custom:9"),
        PanelPlacement::new(DockSide::Left, DockHalf::Primary, 1),
    );
    layout.dock_mut(DockSide::Left).split_permille = 0;
    layout.sanitize();
    // The duplicate is dropped and the orphaned and missing change sections
    // return in factory order.
    assert_eq!(
        ids(layout.sections_in(&p(panel_ids::CHANGES))),
        DEFAULT_CHANGES_SECTIONS.to_vec()
    );
    assert!(layout.sections_in(&p(panel_ids::TERMINAL)).is_empty());
    assert!(layout.placement(&p(panel_ids::REFLOG)).is_some());
    assert!(layout.placement(&p("custom:9")).is_none());
    assert_eq!(
        layout.dock(DockSide::Left).split_permille,
        MIN_SPLIT_PERMILLE
    );
    for section in builtin_sections() {
        assert_ne!(layout.section_location(&section), SectionLocation::Unknown);
    }
}

#[test]
fn visible_sections_follow_panel_visibility() {
    let mut layout = DockLayout::factory();
    let visible = layout.visible_sections();
    assert!(visible.contains(&s(section_ids::REMOTE_BRANCHES)));
    assert!(visible.contains(&s(section_ids::STAGED)));
    assert!(!visible.contains(&s(section_ids::TAGS)));
    layout.toggle_panel(&p(panel_ids::BRANCHES));
    let visible = layout.visible_sections();
    assert!(!visible.contains(&s(section_ids::REMOTE_BRANCHES)));
    assert!(layout.is_section_visible(&s(section_ids::STAGED)));
}

#[test]
fn presets_change_dock_visibility_but_not_placement() {
    let mut layout = DockLayout::factory();
    let custom = layout.add_custom_panel("x", DockSide::Left);
    layout.move_section(&s(section_ids::STAGED), &custom, None);
    assert!(layout.apply_preset(LayoutPreset::Commit));
    assert!(!layout.is_dock_visible(DockSide::Left));
    assert!(layout.is_dock_visible(DockSide::Right));
    assert_eq!(layout.section_panel(&s(section_ids::STAGED)), Some(custom));
    layout.apply_preset(LayoutPreset::Review);
    assert_eq!(layout.centre.mode, CentreSplitMode::Auto);
}

#[test]
fn centre_auto_mode_follows_window_shape() {
    assert_eq!(
        CentreSplitMode::Auto.resolve(1600.0, 900.0),
        CentreSplitMode::SideBySide
    );
    assert_eq!(
        CentreSplitMode::Auto.resolve(900.0, 900.0),
        CentreSplitMode::Stacked
    );
    assert_eq!(
        CentreSplitMode::Single.resolve(1600.0, 900.0),
        CentreSplitMode::Single
    );
}

#[test]
fn named_layouts_save_rename_and_delete() {
    let mut named = NamedLayouts::default();
    let mut layout = DockLayout::factory();
    layout.toggle_hide_all();
    assert_eq!(named.save("  Review ", &layout).as_deref(), Some("Review"));
    assert!(named.get("Review").unwrap().hide_all_stash.is_none());
    assert!(named.save(" ", &layout).is_none());
    assert!(named.rename("Review", "Code review"));
    assert!(!named.rename("missing", "x"));
    assert_eq!(named.names(), vec!["Code review".to_owned()]);
    assert!(named.delete("Code review"));
    assert!(named.names().is_empty());
}

#[test]
fn layouts_round_trip_through_json_and_tolerate_unknown_fields() {
    let mut layout = DockLayout::factory();
    let custom = layout.add_custom_panel("Mine", DockSide::Bottom);
    layout.move_section(&s(section_ids::TAGS), &custom, None);
    layout.centre.mode = CentreSplitMode::SideBySide;
    let json = serde_json::to_string(&layout).unwrap();
    let back: DockLayout = serde_json::from_str(&json).unwrap();
    assert_eq!(back, layout);

    let partial: DockLayout = serde_json::from_str(r#"{"future_field": 1}"#).unwrap();
    assert_eq!(partial, DockLayout::factory());
}

#[test]
fn named_layouts_persist_to_a_file_and_load_back_repaired() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layouts.json");
    assert_eq!(load_named_layouts_from_path(&path), NamedLayouts::default());

    let mut named = NamedLayouts::default();
    let mut layout = DockLayout::factory();
    layout.move_panel(&p(panel_ids::FILES), DockSide::Right, DockHalf::Split, None);
    named.save("Files right", &layout);
    persist_named_layouts_to_path(&named, &path).unwrap();
    assert_eq!(load_named_layouts_from_path(&path), named);

    std::fs::write(&path, b"not json").unwrap();
    assert_eq!(load_named_layouts_from_path(&path), NamedLayouts::default());
}
