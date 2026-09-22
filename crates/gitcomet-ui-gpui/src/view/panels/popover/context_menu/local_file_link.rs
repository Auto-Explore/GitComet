use super::*;

pub(super) fn model(
    repo_id: RepoId,
    source: &gitcomet_core::domain::FileSource,
    path: &std::path::Path,
    missing: bool,
    load_remote_image_url: Option<&str>,
) -> ContextMenuModel {
    model_for_local_file_link(repo_id, source, path, missing, load_remote_image_url)
}

fn model_for_local_file_link(
    repo_id: RepoId,
    source: &gitcomet_core::domain::FileSource,
    path: &std::path::Path,
    missing: bool,
    load_remote_image_url: Option<&str>,
) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header("File".into()),
        // A repository path, so it reads the same on every platform.
        ContextMenuItem::Label(path.to_string_lossy().replace('\\', "/").into()),
        ContextMenuItem::Separator,
    ];
    if let Some(image_url) = load_remote_image_url {
        items.extend([
            ContextMenuItem::Entry {
                label: "Load image".into(),
                icon: Some("icons/refresh.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::LoadRemoteMarkdownImage {
                    url: image_url.to_owned().into(),
                }),
            },
            ContextMenuItem::Separator,
        ]);
    }
    items.push(ContextMenuItem::Entry {
        label: "Open in GitComet".into(),
        icon: Some("icons/file.svg".into()),
        shortcut: None,
        disabled: missing,
        action: Box::new(ContextMenuAction::OpenFileContent {
            repo_id,
            source: source.clone(),
            path: path.to_path_buf(),
        }),
    });
    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::FileSource;

    fn entry_labels(model: &ContextMenuModel) -> Vec<String> {
        model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, .. } => Some(label.to_string()),
                _ => None,
            })
            .collect()
    }

    fn open_entry(model: &ContextMenuModel) -> (&ContextMenuAction, bool) {
        model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry {
                    label,
                    action,
                    disabled,
                    ..
                } if label.as_ref() == "Open in GitComet" => Some((action.as_ref(), *disabled)),
                _ => None,
            })
            .expect("open entry")
    }

    #[test]
    fn model_offers_opening_the_file_and_nothing_else() {
        let path = std::path::Path::new("docs/other.md");
        let model =
            model_for_local_file_link(RepoId(3), &FileSource::WorkingDirectory, path, false, None);

        assert_eq!(entry_labels(&model), vec!["Open in GitComet"]);
        // The resolved path is shown so a link's text cannot disguise where
        // it goes.
        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Label(label) if label.as_ref() == "docs/other.md"
        )));
    }

    #[test]
    fn the_path_label_reads_with_forward_slashes() {
        // Built with `push`, a Windows path shows its native separator; a
        // repository path reads the same everywhere.
        let path = std::path::PathBuf::from("docs\\nested\\other.md");
        let model =
            model_for_local_file_link(RepoId(3), &FileSource::WorkingDirectory, &path, false, None);

        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Label(label) if label.as_ref() == "docs/nested/other.md"
        )));
        // The action keeps the path it was given, separators and all.
        assert!(matches!(
            open_entry(&model).0,
            ContextMenuAction::OpenFileContent { path: action_path, .. } if *action_path == path
        ));
    }

    #[test]
    fn open_entry_carries_the_repo_source_and_path() {
        let path = std::path::Path::new("docs/other.md");
        let source = FileSource::Commit(CommitId("deadbeef".into()));
        let model = model_for_local_file_link(RepoId(3), &source, path, false, None);

        let (action, disabled) = open_entry(&model);
        assert!(!disabled);
        assert!(matches!(
            action,
            ContextMenuAction::OpenFileContent { repo_id, source, path }
                if *repo_id == RepoId(3)
                    && *source == FileSource::Commit(CommitId("deadbeef".into()))
                    && path == std::path::Path::new("docs/other.md")
        ));
    }

    #[test]
    fn a_missing_file_greys_the_entry_out() {
        let path = std::path::Path::new("missing.txt");
        let model =
            model_for_local_file_link(RepoId(3), &FileSource::WorkingDirectory, path, true, None);

        assert_eq!(entry_labels(&model), vec!["Open in GitComet"]);
        assert!(
            open_entry(&model).1,
            "nothing to open, so the entry is disabled"
        );
    }

    #[test]
    fn linked_blocked_image_adds_an_exact_load_action() {
        // A badge wrapped in a local link can only be approved through this
        // menu, exactly as through the web link menu.
        let path = std::path::Path::new("docs/other.md");
        let model = model_for_local_file_link(
            RepoId(3),
            &FileSource::WorkingDirectory,
            path,
            false,
            Some("https://images.example.com/badge.svg"),
        );

        assert_eq!(entry_labels(&model), vec!["Load image", "Open in GitComet"]);
        let load = model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Load image" => {
                    Some(action)
                }
                _ => None,
            })
            .expect("load image entry");
        assert!(matches!(
            load.as_ref(),
            ContextMenuAction::LoadRemoteMarkdownImage { url }
                if url.as_ref() == "https://images.example.com/badge.svg"
        ));
    }
}
