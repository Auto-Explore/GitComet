use super::*;

pub(super) fn model(url: &str) -> ContextMenuModel {
    model_for_markdown_link(url)
}

fn model_for_markdown_link(url: &str) -> ContextMenuModel {
    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Link".into()),
        ContextMenuItem::Label(url.to_owned().into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Open in web browser".into(),
            icon: Some("icons/link.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenWebUrl {
                url: url.to_owned(),
            }),
        },
        ContextMenuItem::Entry {
            label: "Copy link address".into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CopyText {
                text: url.to_owned(),
            }),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn model_offers_opening_and_copying_the_link() {
        let model = model_for_markdown_link("https://example.com/page");

        assert_eq!(
            entry_labels(&model),
            vec!["Open in web browser", "Copy link address"]
        );
        // The destination is shown so a link's text cannot disguise where it goes.
        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Label(label) if label.as_ref() == "https://example.com/page"
        )));
    }

    #[test]
    fn open_entry_carries_the_link_url() {
        let model = model_for_markdown_link("https://example.com/page");
        let open = model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == "Open in web browser" =>
                {
                    Some(action)
                }
                _ => None,
            })
            .expect("open entry");

        assert!(matches!(
            open.as_ref(),
            ContextMenuAction::OpenWebUrl { url } if url == "https://example.com/page"
        ));
    }
}
