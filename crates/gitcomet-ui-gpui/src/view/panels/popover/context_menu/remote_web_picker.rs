use super::*;

/// One row per remote with a web page, `origin` first, so the user picks which
/// to open. Digits pick directly; the first row is preselected for Enter.
pub(super) fn model(this: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let pages = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.remotes.ready())
        .map(|remotes| crate::view::permalink::remote_web_pages(remotes))
        .unwrap_or_default();

    let mut items = vec![
        ContextMenuItem::Header(crate::menu_labels::OPEN_REMOTE_IN_BROWSER.into()),
        ContextMenuItem::Separator,
    ];
    // The remotes can change under an open picker.
    if pages.is_empty() {
        items.push(ContextMenuItem::Label(
            "No remote has a web page to open".into(),
        ));
        return ContextMenuModel::new(items);
    }

    let mut tooltips = FxHashMap::default();
    let mut debug_selectors = FxHashMap::default();
    for (n, page) in pages.into_iter().enumerate() {
        let ix = items.len();
        tooltips.insert(ix, SharedString::from(page.url.clone()));
        debug_selectors.insert(ix, format!("open_remote_in_browser_{n}").into());
        items.push(ContextMenuItem::Entry {
            label: format!("{} — {}", page.remote, page.display_address()).into(),
            icon: Some("icons/cloud.svg".into()),
            shortcut: (n < 9).then(|| (n + 1).to_string().into()),
            disabled: false,
            action: Box::new(ContextMenuAction::OpenWebUrl { url: page.url }),
        });
    }
    ContextMenuModel::new(items)
        .with_entry_tooltips(tooltips)
        .with_entry_debug_selectors(debug_selectors)
}
