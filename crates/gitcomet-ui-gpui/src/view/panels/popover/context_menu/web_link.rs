use super::*;

pub(super) fn model(url: &str, load_remote_image_url: Option<&str>) -> ContextMenuModel {
    model_for_web_link(url, load_remote_image_url)
}

fn model_for_web_link(url: &str, load_remote_image_url: Option<&str>) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header("Link".into()),
        ContextMenuItem::Label(url.to_owned().into()),
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
    items.extend([
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
            action: Box::new(ContextMenuAction::CopyLinkAddress {
                url: url.to_owned(),
            }),
        },
    ]);
    ContextMenuModel::new(items)
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
        let model = model_for_web_link("https://example.com/page", None);

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
        let model = model_for_web_link("https://example.com/page", None);
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

    #[test]
    fn copy_entry_copies_the_address_and_says_so() {
        // A link's address is never on screen — the document shows its text —
        // so the copy has to announce itself. That is what separates this from
        // the plain `CopyText` every other menu uses.
        let model = model_for_web_link("https://example.com/page", None);
        let copy = model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == "Copy link address" =>
                {
                    Some(action)
                }
                _ => None,
            })
            .expect("copy entry");

        assert!(matches!(
            copy.as_ref(),
            ContextMenuAction::CopyLinkAddress { url } if url == "https://example.com/page"
        ));
    }

    #[test]
    fn linked_blocked_image_adds_an_exact_load_action() {
        let model = model_for_web_link(
            "https://example.com/page",
            Some("https://images.example.com/badge.svg"),
        );

        assert_eq!(
            entry_labels(&model),
            vec!["Load image", "Open in web browser", "Copy link address"]
        );
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
