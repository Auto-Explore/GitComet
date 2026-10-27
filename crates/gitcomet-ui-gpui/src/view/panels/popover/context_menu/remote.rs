use super::*;

pub(super) fn model(this: &PopoverHost, repo_id: RepoId, name: &str) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let shown_in_history = !repo.is_some_and(|repo| repo.history_hidden_remotes().contains(name));
    let remote_url = repo
        .and_then(|repo| repo.remotes.ready())
        .and_then(|remotes| remotes.iter().find(|remote| remote.name == name))
        .and_then(|remote| remote.url.as_deref());
    let web_url = remote_url.and_then(crate::view::permalink::remote_web_url);

    let mut items = vec![ContextMenuItem::Header("Remote".into())];
    items.push(ContextMenuItem::Label(name.to_owned().into()));
    items.push(ContextMenuItem::Separator);
    for (label, icon, collapsed) in [
        ("Expand all", "icons/arrow_down_to_line.svg", false),
        ("Collapse all", "icons/arrow_up_to_line.svg", true),
    ] {
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some(icon.into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetBranchGroupCollapsedRecursive {
                section: BranchSection::Remote,
                remote: Some(name.to_owned()),
                path: String::new(),
                collapsed,
            }),
        });
    }
    items.push(ContextMenuItem::Separator);
    let (history_label, history_icon) = if shown_in_history {
        ("Hide from history graph", "icons/eye_off.svg")
    } else {
        ("Show in history graph", "icons/eye.svg")
    };
    items.push(ContextMenuItem::Entry {
        label: history_label.into(),
        icon: Some(history_icon.into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::SetRemoteHistoryVisibility {
            repo_id,
            remote: name.to_owned(),
            hidden: shown_in_history,
        }),
    });
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Fetch all".into(),
        icon: Some("icons/arrow_down.svg".into()),
        shortcut: Some("F".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::FetchAll { repo_id }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Prune merged branches".into(),
        icon: Some("icons/broom.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::PruneMergedBranches { repo_id }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Prune local tags".into(),
        icon: Some("icons/tag.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::PruneLocalTags { repo_id }),
    });
    items.push(ContextMenuItem::Separator);
    // Focusing a remote seeds the walk from every branch it tracks, so the graph
    // shows that remote's work and nothing else.
    items.push(super::focus::entry(
        this,
        repo_id,
        gitcomet_core::domain::HistoryFocus::remote(name),
    ));
    items.push(ContextMenuItem::Separator);

    let open_in_browser_ix = items.len();
    let open_in_browser_tooltip = match (&web_url, remote_url) {
        (Some(url), _) => url.clone(),
        (None, Some(_)) => "This remote's URL doesn't point to a web page".to_owned(),
        (None, None) => "This remote has no URL".to_owned(),
    };
    items.push(ContextMenuItem::Entry {
        label: "Open in web browser".into(),
        icon: Some("icons/link.svg".into()),
        shortcut: None,
        disabled: web_url.is_none(),
        action: Box::new(ContextMenuAction::OpenWebUrl {
            url: web_url.unwrap_or_default(),
        }),
    });

    for (label, kind) in [
        ("Edit fetch URL…", RemoteUrlKind::Fetch),
        ("Edit push URL…", RemoteUrlKind::Push),
    ] {
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some("icons/pencil.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::remote(
                    repo_id,
                    RemotePopoverKind::EditUrlPrompt {
                        name: name.to_owned(),
                        kind,
                    },
                ),
            }),
        });
    }

    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Remove remote…".into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::remote(
                repo_id,
                RemotePopoverKind::RemoveConfirm {
                    name: name.to_owned(),
                },
            ),
        }),
    });

    ContextMenuModel::new(items).with_entry_tooltips(FxHashMap::from_iter([(
        open_in_browser_ix,
        SharedString::from(open_in_browser_tooltip),
    )]))
}
