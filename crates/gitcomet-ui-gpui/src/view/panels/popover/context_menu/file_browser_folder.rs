use super::*;
use gitcomet_core::domain::FileSource;

/// Context menu for a folder row in the sidebar file browser.
///
/// The file menu's counterpart: same header/path block, but the actions are
/// about the folder as a container — opening and closing it in the tree, and
/// handing its location to the OS.
pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    path: &std::path::Path,
) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let source = repo
        .map(|repo| repo.file_browser.source.clone())
        .unwrap_or_default();
    // `Arc<PathBuf>` borrows as `PathBuf` but not as `Path`, so a caller holding
    // a `&Path` has to hand the set an owned copy. Only the `PathBuf` though —
    // wrapping it in an `Arc` as well would allocate a control block per repaint
    // to answer one lookup, and the set never sees the `Arc`.
    let is_expanded = repo.is_some_and(|repo| {
        repo.file_browser
            .expanded_dirs
            .contains(&path.to_path_buf())
    });
    // While a search filters the tree, every directory renders force-expanded
    // and `expanded_dirs` is ignored entirely — the reducer freezes the set for
    // exactly that reason. Offer the toggles as unavailable rather than as
    // controls that silently do nothing.
    //
    // The predicate has to be the tree's own: a whitespace-only query is stored
    // verbatim but produces no matchers, so the tree is *not* filtered and the
    // toggles must stay live.
    let filtered_by_search = repo.is_some_and(|repo| {
        crate::view::panes::file_browser_search_is_active(&repo.file_browser.search_query)
    });

    let mut items = vec![ContextMenuItem::Header(
        path.file_name()
            .and_then(|p| p.to_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{path:?}"))
            .into(),
    )];
    items.push(ContextMenuItem::Label(
        components::ContextMenuText::path_single_line(path.display().to_string()),
    ));
    items.push(ContextMenuItem::Separator);

    items.push(ContextMenuItem::Entry {
        label: if is_expanded { "Collapse" } else { "Expand" }.into(),
        icon: Some(
            if is_expanded {
                "icons/chevron_down.svg"
            } else {
                "icons/chevron_right.svg"
            }
            .into(),
        ),
        shortcut: None,
        disabled: filtered_by_search,
        action: Box::new(ContextMenuAction::ToggleFileBrowserDir {
            repo_id,
            path: path.to_path_buf(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Expand all under here".into(),
        icon: Some("icons/arrow_down_to_line.svg".into()),
        shortcut: None,
        disabled: filtered_by_search,
        action: Box::new(ContextMenuAction::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: path.to_path_buf(),
            expanded: true,
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Collapse all under here".into(),
        icon: Some("icons/arrow_up_to_line.svg".into()),
        shortcut: None,
        disabled: filtered_by_search,
        action: Box::new(ContextMenuAction::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: path.to_path_buf(),
            expanded: false,
        }),
    });

    // A folder listed from a commit or a branch has no guaranteed counterpart
    // on disk, so the OS actions are working-tree only — the same line the file
    // menu draws.
    if matches!(source, FileSource::WorkingDirectory) {
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::Entry {
            label: "Open folder location".into(),
            icon: Some("icons/folder.svg".into()),
            shortcut: None,
            disabled: false,
            // `open_file_location` opens a directory rather than revealing it in
            // its parent, so this lands inside the folder.
            action: Box::new(ContextMenuAction::OpenFileLocation {
                repo_id,
                path: path.to_path_buf(),
            }),
        });
        // Same gate every other menu applies: without a configured editor the
        // entry can only ever produce a "not configured" error toast.
        if crate::external_editor::configured_setting().is_some() {
            items.push(ContextMenuItem::Entry {
                label: "Open in code editor".into(),
                icon: Some("icons/open_external.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::OpenInCodeEditor {
                    repo_id: Some(repo_id),
                    path: path.to_path_buf(),
                }),
            });
        }
    }

    items.push(ContextMenuItem::Separator);
    push_copy_path_entries(&mut items, this, repo_id, path, None);

    ContextMenuModel::new(items)
}
