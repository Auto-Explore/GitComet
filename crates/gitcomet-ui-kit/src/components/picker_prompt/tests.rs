use super::*;

fn sectioned_items() -> Vec<PickerPromptItem> {
    vec![
        PickerPromptItem::plain("pinned-repo").section("Pinned"),
        PickerPromptItem::plain("open-repo").section("Open Repositories"),
        PickerPromptItem::plain("closed-one").section("Recently Closed"),
        PickerPromptItem::plain("closed-two").section("Recently Closed"),
    ]
}

fn header_labels(layout: &PickerPromptLayout) -> Vec<(usize, &str, bool, usize)> {
    layout
        .headers
        .iter()
        .map(|(slot, header)| {
            (
                *slot,
                header.label.as_ref(),
                header.collapsed,
                header.hidden_count,
            )
        })
        .collect()
}

#[test]
fn an_uncollapsed_layout_places_one_header_before_each_sections_first_row() {
    let items = sectioned_items();
    let layout = picker_prompt_layout(&items, "");

    assert_eq!(layout.item_indices, vec![0, 1, 2, 3]);
    // Every header takes a scroll child slot ahead of its first row.
    assert_eq!(layout.child_indices, vec![1, 3, 5, 6]);
    assert_eq!(
        header_labels(&layout),
        vec![
            (0, "Pinned", false, 0),
            (1, "Open Repositories", false, 0),
            (2, "Recently Closed", false, 0),
        ]
    );
    assert!(
        layout
            .headers
            .iter()
            .all(|(slot, _)| *slot < layout.item_indices.len()),
        "with nothing collapsed every header still introduces a row"
    );
}

#[test]
fn a_collapsed_section_keeps_its_header_and_drops_its_rows() {
    let items = sectioned_items();
    let collapsed = BTreeSet::from([SharedString::from("Recently Closed")]);
    let layout = picker_prompt_layout_with_collapsed(&items, "", &collapsed);

    assert_eq!(
        layout.item_indices,
        vec![0, 1],
        "the collapsed section's rows are gone, so keyboard navigation skips them"
    );
    assert_eq!(layout.child_indices, vec![1, 3]);
    assert_eq!(
        header_labels(&layout),
        vec![
            (0, "Pinned", false, 0),
            (1, "Open Repositories", false, 0),
            // Its rows are gone, so the header trails the last visible row.
            (2, "Recently Closed", true, 2),
        ]
    );
}

#[test]
fn collapsing_every_section_leaves_the_headers_as_the_only_rows() {
    let items = sectioned_items();
    let collapsed = BTreeSet::from([
        SharedString::from("Pinned"),
        SharedString::from("Open Repositories"),
        SharedString::from("Recently Closed"),
    ]);
    let layout = picker_prompt_layout_with_collapsed(&items, "", &collapsed);

    assert!(layout.item_indices.is_empty());
    assert_eq!(
        header_labels(&layout),
        vec![
            (0, "Pinned", true, 1),
            (0, "Open Repositories", true, 1),
            (0, "Recently Closed", true, 2),
        ],
        "with no rows at all every header trails slot 0"
    );
    assert!(
        layout
            .headers
            .first()
            .is_some_and(|(_, header)| header.is_first),
        "only the first header skips the rule above it"
    );
    assert!(
        layout.headers[1..]
            .iter()
            .all(|(_, header)| !header.is_first)
    );
}

#[test]
fn a_collapsed_section_still_counts_only_the_rows_the_query_matched() {
    let items = sectioned_items();
    let collapsed = BTreeSet::from([SharedString::from("Recently Closed")]);
    let layout = picker_prompt_layout_with_collapsed(&items, "closed-one", &collapsed);

    assert!(layout.item_indices.is_empty());
    assert_eq!(
        header_labels(&layout),
        vec![(0, "Recently Closed", true, 1)],
        "sections with no matches contribute no header, and the count is of matches"
    );
}

#[test]
fn match_items_skips_queries_longer_than_candidate_labels() {
    let items = vec![
        PickerPromptItem::plain("ab"),
        PickerPromptItem::plain("alphabet"),
    ];

    let matches = match_items(
        &items,
        &section_groups(&items),
        "alphabet soup",
        PickerPromptOrder::Relevance,
    );

    assert!(matches.is_empty());
}

#[test]
fn ascii_matcher_returns_none_when_needle_is_longer_than_haystack() {
    let needle = b"alphabet soup";

    let range = find_ascii_case_insensitive_precomputed(
        b"ab",
        needle,
        needle[0].to_ascii_lowercase(),
        needle[0].to_ascii_uppercase(),
    );

    assert_eq!(range, None);
}

#[test]
fn picker_prompt_item_maps_search_hits_into_part_local_ranges() {
    let item = PickerPromptItem::from_parts([
        PickerPromptItemPart::new("feature/worktree").flexible(false),
        PickerPromptItemPart::separator("  "),
        PickerPromptItemPart::path("/tmp/repo/src/main.rs"),
    ]);

    let matches = match_items(
        std::slice::from_ref(&item),
        &[0],
        "main",
        PickerPromptOrder::Relevance,
    );
    let range = matches
        .first()
        .and_then(|m| m.range.clone())
        .expect("expected a match");

    assert_eq!(item.parts()[0].local_match_range(Some(&range)), None);
    assert_eq!(item.parts()[1].local_match_range(Some(&range)), None);
    assert_eq!(
        item.parts()[2].local_match_range(Some(&range)),
        Some(14..18)
    );
}

#[test]
fn the_marked_row_shows_a_check_instead_of_its_own_icon() {
    let item = PickerPromptItem::plain("main").icon("icons/git_branch.svg");

    assert_eq!(
        row_leading_icon(&item, None, true),
        Some("icons/check.svg"),
        "the marked row gives up its icon for the check"
    );
    assert_eq!(
        row_leading_icon(&item, None, false),
        Some("icons/git_branch.svg")
    );
}

#[test]
fn unmarked_rows_without_an_icon_fall_back_to_the_picker_wide_one() {
    let item = PickerPromptItem::plain("main");

    assert_eq!(
        row_leading_icon(&item, Some("icons/git_branch.svg"), false),
        Some("icons/git_branch.svg")
    );
    assert_eq!(row_leading_icon(&item, None, false), None);
}

#[test]
fn secondary_parts_keep_filtering_and_highlighting_the_row() {
    // The worktree picker moved the path onto the second line; a path query
    // must still find the row and light up the matched span there.
    let item = PickerPromptItem::from_parts([PickerPromptItemPart::new("feature").flexible(false)])
        .secondary_parts([PickerPromptItemPart::path("/tmp/ws/feature/src/main.rs")]);

    let matches = match_items(
        std::slice::from_ref(&item),
        &[0],
        "src",
        PickerPromptOrder::Relevance,
    );
    let range = matches
        .first()
        .and_then(|m| m.range.clone())
        .expect("a path query must still match");

    assert_eq!(item.parts()[0].local_match_range(Some(&range)), None);
    assert_eq!(
        item.secondary()[0].local_match_range(Some(&range)),
        Some(16..19),
        "the hit belongs to the secondary part that contains it"
    );
}

#[test]
fn secondary_parts_stay_out_of_the_sort_key() {
    // Sorting is by what the eye reads first, so a long detail line must not
    // push a short-titled row down the list.
    let items = vec![
        PickerPromptItem::from_parts([PickerPromptItemPart::new("main")]).secondary_parts([
            PickerPromptItemPart::path("/a/very/long/path/that/dwarfs/the/title"),
        ]),
        PickerPromptItem::from_parts([PickerPromptItemPart::new("maintenance")]),
    ];

    let matches = match_items(
        &items,
        &section_groups(&items),
        "main",
        PickerPromptOrder::Relevance,
    );

    assert_eq!(
        matches.iter().map(|m| m.index).collect::<Vec<_>>(),
        vec![0, 1],
        "the shorter title sorts first regardless of its detail line"
    );
}

#[test]
fn picker_prompt_layout_keeps_sections_contiguous_when_filtering() {
    let items = vec![
        PickerPromptItem::plain("zulu-repo").section("Open"),
        PickerPromptItem::plain("repo-one").section("Recently Closed"),
        PickerPromptItem::plain("repo-two").section("Recently Closed"),
    ];

    let layout = picker_prompt_layout(&items, "repo");

    // The "Recently Closed" hits sort earlier on match position, but must
    // not be hoisted above the "Open" section.
    assert_eq!(layout.item_indices, vec![0, 1, 2]);
    assert_eq!(layout.child_indices, vec![1, 3, 4]);
}

#[test]
fn picker_prompt_layout_drops_headers_for_sections_without_matches() {
    let items = vec![
        PickerPromptItem::plain("alpha").section("Open"),
        PickerPromptItem::plain("gamma").section("Recently Closed"),
    ];

    let layout = picker_prompt_layout(&items, "gam");

    assert_eq!(layout.item_indices, vec![1]);
    assert_eq!(layout.child_indices, vec![1]);
}

fn two_line_items(count: usize) -> Vec<PickerPromptItem> {
    (0..count)
        .map(|ix| {
            PickerPromptItem::plain(format!("branch-{ix}"))
                .secondary_parts([PickerPromptItemPart::new("Ada  •  2 days ago")])
        })
        .collect()
}

fn geometry_for(items: &[PickerPromptItem]) -> PickerPromptGeometry {
    let layout = picker_prompt_layout(items, "");
    PickerPromptGeometry::new(items, &layout, UiScale::from_percent(100))
}

#[test]
fn a_list_that_barely_scrolls_renders_every_row() {
    // Windowing only earns its keep on long lists; short ones keep exactly
    // the geometry they had before it existed.
    let items = two_line_items(8);
    let geometry = geometry_for(&items);
    let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);

    let window = geometry.window(px(0.0), viewport);

    assert!(!geometry.is_windowed(viewport));
    assert_eq!(window.rows, 0..8);
    assert_eq!(window.space_before, px(0.0));
    assert_eq!(window.space_after, px(0.0));
}

#[test]
fn a_long_list_renders_only_what_can_be_seen() {
    let items = two_line_items(1_200);
    let geometry = geometry_for(&items);
    let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);

    let window = geometry.window(px(0.0), viewport);

    assert!(geometry.is_windowed(viewport));
    // A 300px viewport over 50px rows is six rows, plus the overdraw below.
    assert!(
        window.rows.len() < 20,
        "expected a viewport-sized window, rendered {} rows",
        window.rows.len()
    );
    assert_eq!(window.rows.start, 0);
}

#[test]
fn the_windowed_list_is_exactly_as_tall_as_the_full_one() {
    // The spacers stand in for the rows outside the window — and only for
    // those: the list's own padding stays the list's, or the rows would shift
    // down and the scrollbar would grow the moment windowing kicked in. If
    // this did not add up, scrolling would jump as the window moved.
    let items = two_line_items(1_200);
    let geometry = geometry_for(&items);
    let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);
    let pad = UiScale::from_percent(100).px(LIST_PAD_PX);

    for offset in [px(0.0), px(640.0), px(20_000.0)] {
        let window = geometry.window(offset, viewport);
        let rendered: Pixels = window
            .rows
            .clone()
            .map(|ix| geometry.heights[ix] + geometry.header_heights[ix])
            .fold(px(0.0), |sum, height| sum + height);

        assert_eq!(
            pad + window.space_before + rendered + window.space_after + pad,
            geometry.total_height(),
            "spacers must account for every row outside the window at {offset:?}"
        );
    }
    // And an unwindowed list adds no spacers at all.
    let short = geometry_for(&two_line_items(4));
    let window = short.window(px(0.0), viewport);
    assert_eq!(window.space_before, px(0.0));
    assert_eq!(window.space_after, px(0.0));
}

#[test]
fn scrolling_down_keeps_the_window_around_the_viewport() {
    let items = two_line_items(1_200);
    let geometry = geometry_for(&items);
    let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);
    let offset = geometry.reveal_offset(600, viewport, px(0.0));

    let window = geometry.window(offset, viewport);

    assert!(window.rows.contains(&600));
    assert!(window.rows.start > 0, "rows above the viewport are dropped");
    assert!(window.space_before > px(0.0));
    assert!(window.space_after > px(0.0));
}

#[test]
fn reveal_offset_scrolls_to_rows_outside_the_viewport_and_leaves_visible_ones_alone() {
    let items = two_line_items(1_200);
    let geometry = geometry_for(&items);
    let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);

    // Offsets are in the scroll container's coordinates, so they count the
    // list's top padding as well as the rows above.
    let pad = UiScale::from_percent(100).px(LIST_PAD_PX);

    // A row far below: scrolled to sit at the bottom edge of the viewport.
    let down = geometry.reveal_offset(40, viewport, px(0.0));
    assert_eq!(
        down,
        pad + geometry.row_top(40) + geometry.row_height(40) - viewport
    );

    // A row above the current offset: scrolled to the top edge.
    let up = geometry.reveal_offset(10, viewport, down);
    assert_eq!(up, pad + geometry.row_top(10));

    // A row already fully visible: left where it is.
    let unchanged = geometry.reveal_offset(1, viewport, px(0.0));
    assert_eq!(unchanged, px(0.0));
}

#[test]
fn reveal_offset_stays_within_the_scrollable_range() {
    let items = two_line_items(1_200);
    let geometry = geometry_for(&items);
    let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);

    let last = geometry.reveal_offset(1_199, viewport, px(0.0));

    // Scrolls no further than it has to: the last row sits flush with the
    // bottom edge, which is inside the scrollable range (the list's bottom
    // padding is the remaining few pixels).
    let pad = UiScale::from_percent(100).px(LIST_PAD_PX);
    assert_eq!(
        last,
        pad + geometry.row_top(1_199) + geometry.row_height(1_199) - viewport
    );
    assert!(last <= geometry.total_height() - viewport);
}

#[test]
fn a_section_header_is_counted_once_above_the_row_that_opens_it() {
    let items = vec![
        PickerPromptItem::plain("alpha").section("Local Branches"),
        PickerPromptItem::plain("beta").section("Local Branches"),
        PickerPromptItem::plain("origin/gamma").section("Remote Branches"),
    ];
    let geometry = geometry_for(&items);
    let ui_scale = UiScale::from_percent(100);

    assert_eq!(
        geometry.header_heights[0],
        section_header_height(ui_scale, true)
    );
    assert_eq!(geometry.header_heights[1], px(0.0));
    assert_eq!(
        geometry.header_heights[2],
        section_header_height(ui_scale, false),
        "every header but the first also carries its rule"
    );
    // The second section's rows sit below both headers and the rows above.
    let single = row_height(ui_scale, false);
    assert_eq!(
        geometry.row_top(2),
        section_header_height(ui_scale, true)
            + single * 2.0
            + section_header_height(ui_scale, false)
    );
}

/// The geometry caches are keyed on this, so a scale it cannot round-trip
/// means every render rebuilds the whole table.
#[test]
fn geometry_reports_the_scale_it_was_built_at() {
    let items = [PickerPromptItem::plain("only")];
    let layout = picker_prompt_layout(&items, "");

    for density in crate::appearance::UiDensity::ALL {
        for percent in [100, 150] {
            let scale =
                UiScale::from_percent(percent).with_appearance(crate::appearance::Appearance {
                    density,
                    ..crate::appearance::Appearance::default()
                });

            let geometry = PickerPromptGeometry::new(&items, &layout, scale);

            assert_eq!(
                geometry.ui_scale(),
                scale,
                "geometry built at {percent}% {density:?} must compare equal to it"
            );
        }
    }
}

/// Why the round-trip matters: Comfortable changes the heights.
#[test]
fn comfortable_density_makes_picker_rows_taller() {
    let compact = UiScale::from_percent(100).with_appearance(crate::appearance::Appearance {
        density: crate::appearance::UiDensity::Compact,
        ..crate::appearance::Appearance::default()
    });
    let comfortable = compact.with_appearance(crate::appearance::Appearance {
        density: crate::appearance::UiDensity::Comfortable,
        ..crate::appearance::Appearance::default()
    });

    assert!(row_height(comfortable, false) > row_height(compact, false));
}

#[test]
fn a_detail_line_adds_one_line_box_when_content_determines_row_height() {
    let ui_scale = UiScale::from_percent(100).with_appearance(crate::appearance::Appearance {
        density: crate::appearance::UiDensity::Compact,
        ..crate::appearance::Appearance::default()
    });

    assert_eq!(
        row_height(ui_scale, true) - row_height(ui_scale, false),
        text_line_height(ui_scale, SECONDARY_TEXT_REMS)
    );
}

#[test]
fn a_single_line_row_is_never_shorter_than_a_control() {
    // `min_h(control_height_md)` is what stops a one-line row from collapsing.
    let ui_scale = UiScale::from_percent(80);

    assert!(row_height(ui_scale, false) >= control_height_md(ui_scale));
}

#[test]
fn picker_prompt_item_search_skips_non_searchable_separators() {
    let item = PickerPromptItem::from_parts([
        PickerPromptItemPart::new("repo").flexible(false),
        PickerPromptItemPart::separator(" - "),
        PickerPromptItemPart::path("/tmp/workspace"),
    ]);

    let matches = match_items(&[item], &[0], " - ", PickerPromptOrder::Relevance);

    assert!(matches.is_empty());
}
#[test]
fn source_order_keeps_matches_in_declaration_order() {
    let items = vec![
        PickerPromptItem::plain("long matching summary"),
        PickerPromptItem::plain("summary"),
    ];
    assert_eq!(
        picker_prompt_layout(&items, "summary").item_indices,
        vec![1, 0]
    );
    assert_eq!(
        picker_prompt_layout_ordered(
            &items,
            "summary",
            &BTreeSet::new(),
            PickerPromptOrder::Source
        )
        .item_indices,
        vec![0, 1]
    );
}

#[test]
fn window_agrees_with_a_linear_scan_at_every_offset() {
    let items: Vec<_> = two_line_items(1200)
        .into_iter()
        .enumerate()
        .map(|(ix, item)| {
            if ix < 500 {
                item.section("Recent")
            } else {
                item.section("Older")
            }
        })
        .collect();
    for scale in [100u32, 125, 175] {
        let geometry = PickerPromptGeometry::new(&items, &picker_prompt_layout(&items, ""), scale);
        let viewport = UiScale::from_percent(scale).px(340.0);
        let max_offset = f32::from(geometry.total_height() - viewport) as usize;
        for offset in 0..=max_offset {
            let top = (px(offset as f32) - geometry.pad).max(px(0.0));
            let bottom = top + viewport;
            let first = (0..items.len())
                .find(|ix| geometry.tops[*ix] + geometry.heights[*ix] > top)
                .unwrap();
            let last = (first..items.len())
                .take_while(|ix| geometry.tops[*ix] < bottom)
                .last()
                .unwrap_or(first);
            let expected = first.saturating_sub(WINDOW_OVERDRAW_ROWS)
                ..(last + WINDOW_OVERDRAW_ROWS + 1).min(items.len());
            let actual = geometry.window(px(offset as f32), viewport);
            assert_eq!(actual.rows, expected, "scale={scale} offset={offset}");
        }
        assert_eq!(
            geometry
                .window(geometry.total_height() + px(100.0), viewport)
                .rows
                .end,
            items.len()
        );
    }
}
