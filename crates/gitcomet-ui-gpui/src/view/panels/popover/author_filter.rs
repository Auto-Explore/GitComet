use super::*;
use rustc_hash::FxHashSet;
use std::sync::Arc;

const ALL_AUTHORS_LABEL: &str = "All authors";

/// What activating a row does: clear the filter, or filter to one author.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum AuthorTarget {
    All,
    Author(SharedString),
}

/// Author names from the loaded log commits: trimmed, non-empty, deduplicated
/// case-insensitively and sorted.
///
/// Runs over the whole accumulated log, which grows without bound as pages are
/// appended, so it avoids per-commit allocation: the exact-spelling gate
/// absorbs nearly every commit for the cost of one hash and a refcount bump
/// (the gix backend interns repeated author names, and history comes in runs by
/// the same person), leaving one lowercase key per distinct spelling.
fn collect_author_suggestions(commits: &[gitcomet_core::domain::Commit]) -> Vec<SharedString> {
    let mut seen_spellings: FxHashSet<Arc<str>> = FxHashSet::default();
    let mut seen_folded: FxHashSet<String> = FxHashSet::default();
    let mut authors: Vec<SharedString> = Vec::new();

    for commit in commits {
        if !seen_spellings.insert(commit.author.clone()) {
            continue;
        }
        let name = commit.author.trim();
        if name.is_empty() {
            continue;
        }
        if seen_folded.insert(name.to_ascii_lowercase()) {
            authors.push(SharedString::from(name.to_owned()));
        }
    }

    // ASCII folding throughout — the same rule the dedup above, the active-filter
    // mark below, and the backend's own matcher apply. A Unicode fold here would
    // order names by a key nothing else in the feature agrees with.
    //
    // `sort_by_cached_key` builds one key per author; `sort_by_key` would
    // rebuild it on every comparison.
    authors.sort_by_cached_key(|author| author.to_ascii_lowercase());
    authors
}

/// Author suggestions for `repo_id`, memoized on the repository's log revision.
///
/// The popover re-renders on every mouse move over it, so this must not rescan
/// the log per frame. `log_rev` bumps on every log replacement and is what the
/// popover fingerprint hashes, so the memo and the re-render gate cannot drift.
///
/// Suggestions are only refreshed from an unfiltered, loaded log: once a filter
/// is applied the log holds that author's commits alone, so recomputing would
/// collapse the list to the name already selected and leave no way back to
/// anyone else.
pub(super) fn suggestions(this: &mut PopoverHost, repo_id: RepoId) -> Arc<[SharedString]> {
    let empty: Arc<[SharedString]> = Arc::from(Vec::new());
    let Some(repo) = this.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return empty;
    };

    let log_rev = repo.history_state.log_rev;
    if let Some((cached_repo, cached_rev, cached)) = &this.history_author_suggestions
        && *cached_repo == repo_id
        && *cached_rev == log_rev
    {
        return cached.clone();
    }

    let filtered = repo.history_state.history_author_filter.is_some();
    let cached_for_repo = this
        .history_author_suggestions
        .as_ref()
        .filter(|(cached_repo, ..)| *cached_repo == repo_id)
        .map(|(.., cached)| cached.clone());

    // A filtered log only describes the author already selected, so keep the
    // last list instead — unless there is nothing to keep (a filter restored
    // from the session), where a one-name list still beats an empty one.
    let refresh_from_log = !filtered || cached_for_repo.is_none();
    let authors = match (&repo.history_state.log, refresh_from_log) {
        (Loadable::Ready(page), true) => Arc::from(collect_author_suggestions(&page.commits)),
        // Filtered, or still loading: keep the list the user last saw.
        _ => cached_for_repo.unwrap_or(empty),
    };

    this.history_author_suggestions = Some((repo_id, log_rev, authors.clone()));
    authors
}

/// Every row the dropdown can offer, and the target each one applies.
/// `items` and `targets` are index-aligned, so a `PickerPrompt` selection index
/// (which is the original, pre-filter index) reads straight out of `targets`.
///
/// Narrowing by the typed query is left to [`components::PickerPrompt`], which
/// filters the items it is handed anyway — doing it here first would scan every
/// author twice per keystroke and give the two passes a chance to disagree.
pub(super) struct AuthorRows {
    pub(super) items: Vec<components::PickerPromptItem>,
    pub(super) targets: Vec<AuthorTarget>,
    pub(super) marked_index: Option<usize>,
}

pub(super) fn rows(authors: &[SharedString], current: Option<&str>) -> AuthorRows {
    // "All authors" is always offered; `PickerPrompt` drops it from the
    // rendered rows when it does not match the query, and the navigation
    // targets are derived from that same layout.
    let mut items = Vec::with_capacity(authors.len() + 1);
    let mut targets = Vec::with_capacity(authors.len() + 1);
    items.push(components::PickerPromptItem::from(SharedString::from(
        ALL_AUTHORS_LABEL,
    )));
    targets.push(AuthorTarget::All);
    let mut marked_index = current.is_none().then_some(0);

    for author in authors {
        if marked_index.is_none()
            && current.is_some_and(|current| current.eq_ignore_ascii_case(author))
        {
            marked_index = Some(items.len());
        }
        items.push(components::PickerPromptItem::from(author.clone()));
        targets.push(AuthorTarget::Author(author.clone()));
    }

    AuthorRows {
        items,
        targets,
        marked_index,
    }
}

/// Every row for `repo_id`, with the repository's active filter marked.
fn rows_for_repo(this: &mut PopoverHost, repo_id: RepoId) -> AuthorRows {
    let authors = suggestions(this, repo_id);
    let current = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.history_state.history_author_filter.clone());
    rows(&authors, current.as_deref())
}

/// What the arrow keys walk, in the order the rows are rendered.
///
/// `PickerPrompt` re-sorts a non-empty query's matches by where the match lands,
/// so this has to go through the layout helper — walking `targets` directly
/// would let Enter apply a different author than the highlighted row.
pub(super) fn nav_targets(
    this: &mut PopoverHost,
    repo_id: RepoId,
    query: &str,
) -> Vec<AuthorTarget> {
    let rows = rows_for_repo(this, repo_id);
    let layout = components::picker_prompt_layout(&rows.items, query);
    layout
        .item_indices
        .iter()
        .filter_map(|&ix| rows.targets.get(ix).cloned())
        .collect()
}

/// The rows the dropdown renders for `query`, with the layout that places them.
///
/// The list is windowed, so a row below the viewport has no element for
/// `ScrollHandle::scroll_to_item` to find; keyboard navigation scrolls by this
/// geometry instead. See `PopoverHost::scroll_picker_prompt_to_row`.
pub(super) fn rendered_rows(
    this: &mut PopoverHost,
    repo_id: RepoId,
    query: &str,
) -> (
    Vec<components::PickerPromptItem>,
    components::PickerPromptLayout,
) {
    let rows = rows_for_repo(this, repo_id);
    let layout = components::picker_prompt_layout(&rows.items, query);
    (rows.items, layout)
}

pub(super) fn apply(
    this: &mut PopoverHost,
    repo_id: RepoId,
    target: AuthorTarget,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let author = match target {
        AuthorTarget::All => None,
        AuthorTarget::Author(name) => {
            let name = name.trim();
            (!name.is_empty()).then(|| name.to_owned())
        }
    };
    this.store
        .dispatch(Msg::SetHistoryAuthorFilter { repo_id, author });
    this.close_popover(cx);
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let width = super::HISTORY_AUTHOR_FILTER_WIDTH;

    let Some(search) = this.history_author_filter_search_input.clone() else {
        return components::context_menu(
            theme,
            div().w(width.preferred_px(ui_scale)).child(
                components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    "Search input not initialized",
                    Some(this.tooltip_host.clone()),
                    cx,
                )
                .into_any_element(),
            ),
        );
    };

    // A chromeless input with a leading magnifier, matching the repository
    // picker's search field.
    search.update(cx, |input, cx| {
        input.set_chromeless(true, cx);
        input.set_leading_icon(Some("icons/zoom.svg"), cx);
    });

    let query = search.read(cx).text().trim().to_string();
    let rows = rows_for_repo(this, repo_id);
    let targets = rows.targets;

    // Suggestions only cover the commits loaded so far, so a name that is not
    // in the list is still a valid filter — say so instead of a dead end.
    let empty_text = if query.is_empty() {
        "No authors"
    } else {
        "No match — Enter filters on what you typed"
    };

    // Items and layout together, so the rows rendered and the rows navigation
    // walks are one derivation rather than two that could order differently.
    let layout = std::rc::Rc::new(components::picker_prompt_layout(&rows.items, &query));
    let items: std::rc::Rc<[components::PickerPromptItem]> = rows.items.into();
    let prompt = components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
        .prebuilt_items(items, layout)
        .tooltip_host(this.tooltip_host.clone())
        .empty_text(empty_text)
        .max_height(scaled_px(components::PICKER_LIST_MAX_HEIGHT_PX))
        .selected_index(this.history_author_filter_selected_index)
        .marked_index(rows.marked_index)
        .accent_selection()
        // A busy repository has thousands of contributors, and the list windows
        // itself past a couple of viewports: only the rows on screen are built.
        // Keyboard navigation scrolls by the row geometry to match
        // (`scroll_picker_prompt_to_row`).
        .padded_query_row();

    components::context_menu(
        theme,
        prompt.render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
            let Some(target) = targets.get(ix).cloned() else {
                return;
            };
            apply(this, repo_id, target, cx);
        }),
    )
    // Fixed width: PickerPrompt rows size with `w_full`, which does not stretch
    // under fit-content parents.
    .w(width.preferred_px(ui_scale))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Commit, CommitId, CommitParentIds};

    fn commit(author: &str) -> Commit {
        Commit {
            id: CommitId("deadbeefdeadbeef".into()),
            parent_ids: CommitParentIds::new(),
            summary: "msg".into(),
            author: author.into(),
            time: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    fn labels(rows: &AuthorRows) -> Vec<String> {
        rows.targets.iter().map(target_label).collect()
    }

    /// The rows as the dropdown actually renders them for `query` — narrowing
    /// and match-position ordering both belong to `PickerPrompt`.
    fn rendered_labels(rows: &AuthorRows, query: &str) -> Vec<String> {
        components::picker_prompt_layout(&rows.items, query)
            .item_indices
            .iter()
            .map(|&ix| target_label(&rows.targets[ix]))
            .collect()
    }

    fn target_label(target: &AuthorTarget) -> String {
        match target {
            AuthorTarget::All => ALL_AUTHORS_LABEL.to_owned(),
            AuthorTarget::Author(name) => name.to_string(),
        }
    }

    #[test]
    fn author_suggestions_are_deduplicated_case_insensitively_and_sorted() {
        let authors = collect_author_suggestions(&[
            commit("Bob"),
            commit("Alice"),
            commit("alice"),
            commit("  Bob  "),
            commit(""),
            commit("   "),
        ]);

        assert_eq!(authors, vec!["Alice", "Bob"]);
    }

    #[test]
    fn all_authors_is_first_and_marked_when_no_filter_is_active() {
        let authors: Vec<SharedString> = vec!["Alice".into(), "Bob".into()];
        let rows = rows(&authors, None);

        assert_eq!(labels(&rows), vec!["All authors", "Alice", "Bob"]);
        assert_eq!(rows.marked_index, Some(0));
        assert_eq!(rows.targets[0], AuthorTarget::All);
        assert_eq!(rows.targets[1], AuthorTarget::Author("Alice".into()));
    }

    /// Dedup, ordering and the active-filter mark all fold ASCII only — the rule
    /// the backend's own matcher uses. A Unicode fold in any one of the three
    /// would disagree with the other two: names that show as separate rows would
    /// sort as if they were the same, and the row for the active filter would go
    /// unmarked.
    #[test]
    fn non_ascii_names_are_folded_the_same_way_at_every_step() {
        let authors = collect_author_suggestions(&[
            commit("Zoe"),
            commit("Éric"),
            commit("éric"),
            commit("ÉRIC"),
        ]);

        // "Éric" and "ÉRIC" differ only in ASCII letters, so they collapse; the
        // two accents stay distinct, because nothing downstream — the backend
        // matcher least of all — would treat them as the same person.
        assert_eq!(authors, vec!["Zoe", "Éric", "éric"]);

        let rows = rows(&authors, Some("éric"));
        assert_eq!(
            rows.marked_index,
            Some(3),
            "the row for the active filter has to be the one that is marked"
        );
    }

    #[test]
    fn active_filter_is_marked_case_insensitively() {
        let authors: Vec<SharedString> = vec!["Alice".into(), "Bob".into()];
        let rows = rows(&authors, Some("alice"));

        assert_eq!(rows.marked_index, Some(1));
    }

    #[test]
    fn query_narrows_rendered_rows_case_insensitively() {
        let authors: Vec<SharedString> = vec!["Alice".into(), "Bob".into(), "boberta".into()];
        let rows = rows(&authors, None);

        assert_eq!(rendered_labels(&rows, "BO"), vec!["Bob", "boberta"]);
    }

    /// Every author is offered, however many there are — the list is
    /// virtualized, so a long one costs no more to render than a short one.
    #[test]
    fn every_matching_author_gets_a_row() {
        let authors: Vec<SharedString> = (0..5_000)
            .map(|ix| SharedString::from(format!("author {ix:04}")))
            .collect();
        let rows = rows(&authors, None);

        // "All authors" rides along on top of the full list.
        assert_eq!(rows.items.len(), 5_001);
        assert_eq!(rows.targets.len(), 5_001);
        assert_eq!(
            rows.targets.last(),
            Some(&AuthorTarget::Author("author 4999".into()))
        );
    }

    /// Navigation walks the rendered order, which `PickerPrompt` re-sorts by
    /// match position once a query is present.
    #[test]
    fn nav_order_follows_the_rendered_order() {
        let authors: Vec<SharedString> = vec!["Zoe Bar".into(), "Bar Zoe".into()];
        let rows = rows(&authors, None);

        // "Bar Zoe" matches at offset 0, so it renders above "Zoe Bar";
        // "All authors" does not match and drops out entirely.
        assert_eq!(rendered_labels(&rows, "bar"), vec!["Bar Zoe", "Zoe Bar"]);
    }
}
