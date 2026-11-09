use super::*;

fn row(path: &str, recency: usize) -> SortableRow {
    sortable_row(
        RepoPickerEntry::Closed(std::path::PathBuf::from(path)),
        std::path::Path::new(path),
        RECENTLY_CLOSED_SECTION,
        recency,
        true,
    )
}

fn names(rows: &[SortableRow]) -> Vec<&str> {
    rows.iter().map(|row| row.name_key.as_str()).collect()
}

fn fixture() -> Vec<SortableRow> {
    vec![
        row("/tmp/b-parent/Alpha", 0),
        row("/tmp/a-parent/zulu", 1),
        row("/tmp/c-parent/mike", 2),
    ]
}

#[test]
fn newest_and_oldest_follow_recency_rank() {
    let mut rows = fixture();
    sort_rows(&mut rows, RepoPickerSort::Newest);
    assert_eq!(names(&rows), vec!["alpha", "zulu", "mike"]);

    sort_rows(&mut rows, RepoPickerSort::Oldest);
    assert_eq!(names(&rows), vec!["mike", "zulu", "alpha"]);
}

#[test]
fn name_sort_is_case_insensitive_and_ignores_parent_directories() {
    let mut rows = fixture();
    sort_rows(&mut rows, RepoPickerSort::Name);
    assert_eq!(names(&rows), vec!["alpha", "mike", "zulu"]);
}

#[test]
fn path_sort_orders_by_full_path_not_repo_name() {
    let mut rows = fixture();
    sort_rows(&mut rows, RepoPickerSort::Path);
    assert_eq!(names(&rows), vec!["zulu", "alpha", "mike"]);
}

#[test]
fn sort_round_trips_through_its_storage_key() {
    for sort in RepoPickerSort::ALL {
        assert_eq!(
            RepoPickerSort::from_storage_key(sort.storage_key()),
            Some(sort)
        );
    }
    assert_eq!(RepoPickerSort::from_storage_key("nonsense"), None);
}

#[test]
fn sort_toggle_includes_the_selected_sort() {
    assert_eq!(sort_toggle_label(RepoPickerSort::Newest), "Sort: Newest");
    assert_eq!(sort_toggle_label(RepoPickerSort::Oldest), "Sort: Oldest");
    assert_eq!(
        sort_toggle_label(RepoPickerSort::Name),
        format!("Sort: {}", RepoPickerSort::Name.label())
    );
    assert_eq!(
        sort_toggle_label(RepoPickerSort::Path),
        format!("Sort: {}", RepoPickerSort::Path.label())
    );
}

#[test]
fn review_regression_selected_sort_applies_to_workspace_rows() {
    let mut zulu = session::Workspace::new(vec!["/repos/zulu".into()]);
    zulu.custom_name = Some("Zulu".to_string());
    zulu.last_activation_order = 20;
    let mut alpha = session::Workspace::new(vec!["/repos/alpha".into()]);
    alpha.custom_name = Some("Alpha".to_string());
    alpha.last_activation_order = 10;

    let rows = workspace_rows(vec![zulu, alpha], RepoPickerSort::Name);

    assert_eq!(names(&rows), vec!["alpha", "zulu"]);
}
