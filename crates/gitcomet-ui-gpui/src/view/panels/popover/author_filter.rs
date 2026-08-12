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

    // `sort_by_cached_key` builds one key per author; `sort_by_key` would
    // rebuild it on every comparison.
    authors.sort_by_cached_key(|author| author.to_lowercase());
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

/// Case-insensitive substring match, folding ASCII only — the same rule
/// [`components::PickerPrompt`] applies when it filters the rows it is handed.
fn matches_query(name: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let needle = query.as_bytes();
    let haystack = name.as_bytes();
    haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

/// The rows the dropdown shows for `query`, and the target each one applies.
/// `items` and `targets` are index-aligned, so a `PickerPrompt` selection index
/// (which is the original, pre-filter index) reads straight out of `targets`.
pub(super) struct AuthorRows {
    pub(super) items: Vec<components::PickerPromptItem>,
    pub(super) targets: Vec<AuthorTarget>,
    pub(super) marked_index: Option<usize>,
}

pub(super) fn rows(authors: &[SharedString], current: Option<&str>, query: &str) -> AuthorRows {
    // "All authors" is always offered; `PickerPrompt` drops it from the
    // rendered rows when it does not match the query, and the navigation
    // targets are derived from that same layout.
    let mut items = vec![components::PickerPromptItem::from(SharedString::from(
        ALL_AUTHORS_LABEL,
    ))];
    let mut targets = vec![AuthorTarget::All];
    let mut marked_index = current.is_none().then_some(0);

    for author in authors.iter().filter(|author| matches_query(author, query)) {
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
    let authors = suggestions(this, repo_id);
    let current = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.history_state.history_author_filter.clone());
    let rows = rows(&authors, current.as_deref(), query);
    components::picker_prompt_layout(&rows.items, query)
        .item_indices
        .iter()
        .filter_map(|&ix| rows.targets.get(ix).cloned())
        .collect()
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
    let authors = suggestions(this, repo_id);
    let current = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.history_state.history_author_filter.clone());
    let rows = rows(&authors, current.as_deref(), &query);
    let targets = rows.targets;

    // Suggestions only cover the commits loaded so far, so a name that is not
    // in the list is still a valid filter — say so instead of a dead end.
    let empty_text = if query.is_empty() {
        "No authors"
    } else {
        "No match — Enter filters on what you typed"
    };

    let prompt = components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
        .items(rows.items)
        .tooltip_host(this.tooltip_host.clone())
        .empty_text(empty_text)
        .max_height(scaled_px(320.0))
        .selected_index(this.history_author_filter_selected_index)
        .marked_index(rows.marked_index)
        .accent_selection()
        .padded_query_row()
        // A busy repository has thousands of contributors: build only the rows
        // on screen, so every author stays scrollable without the whole list
        // being laid out each frame.
        .virtualized(this.history_author_filter_list_scroll.clone());

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
        let rows = rows(&authors, None, "");

        assert_eq!(labels(&rows), vec!["All authors", "Alice", "Bob"]);
        assert_eq!(rows.marked_index, Some(0));
        assert_eq!(rows.targets[0], AuthorTarget::All);
        assert_eq!(rows.targets[1], AuthorTarget::Author("Alice".into()));
    }

    #[test]
    fn active_filter_is_marked_case_insensitively() {
        let authors: Vec<SharedString> = vec!["Alice".into(), "Bob".into()];
        let rows = rows(&authors, Some("alice"), "");

        assert_eq!(rows.marked_index, Some(1));
    }

    #[test]
    fn query_narrows_rows_case_insensitively() {
        let authors: Vec<SharedString> = vec!["Alice".into(), "Bob".into(), "boberta".into()];
        let rows = rows(&authors, None, "BO");

        assert_eq!(labels(&rows), vec!["All authors", "Bob", "boberta"]);
    }

    /// Every author is offered, however many there are — the list is
    /// virtualized, so a long one costs no more to render than a short one.
    #[test]
    fn every_matching_author_gets_a_row() {
        let authors: Vec<SharedString> = (0..5_000)
            .map(|ix| SharedString::from(format!("author {ix:04}")))
            .collect();
        let rows = rows(&authors, None, "");

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
        let rows = rows(&authors, None, "bar");
        let ordered: Vec<String> = components::picker_prompt_layout(&rows.items, "bar")
            .item_indices
            .iter()
            .map(|&ix| target_label(&rows.targets[ix]))
            .collect();

        // "Bar Zoe" matches at offset 0, so it renders above "Zoe Bar";
        // "All authors" does not match and drops out entirely.
        assert_eq!(ordered, vec!["Bar Zoe", "Zoe Bar"]);
    }
}
