use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

pub struct GitCometAssets;

impl GitCometAssets {
    fn load_static(path: &str) -> Option<Cow<'static, [u8]>> {
        match path {
            "gitcomet-window-icon.png" => Some(Cow::Borrowed(include_bytes!(
                "../../../assets/gitcomet-window-icon.png"
            ))),
            "gitcomet-512.png" => Some(Cow::Borrowed(include_bytes!(
                "../../../assets/gitcomet-512.png"
            ))),
            "gitcomet_logo.svg" => Some(Cow::Borrowed(include_bytes!(
                "../../../assets/gitcomet_logo.svg"
            ))),
            "icons/arrow_down.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/arrow_down.svg"
            ))),
            "icons/arrow_up.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/arrow_up.svg"
            ))),
            "icons/spinner.svg" => {
                Some(Cow::Borrowed(include_bytes!("../assets/icons/spinner.svg")))
            }
            "icons/box.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/box.svg"))),
            "icons/check.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/check.svg"))),
            "icons/chevron_down.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/chevron_down.svg"
            ))),
            "icons/plus.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/plus.svg"))),
            "icons/minus.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/minus.svg"))),
            "icons/question.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/question.svg"
            ))),
            "icons/warning.svg" => {
                Some(Cow::Borrowed(include_bytes!("../assets/icons/warning.svg")))
            }
            "icons/swap.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/swap.svg"))),
            "icons/open_external.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/open_external.svg"
            ))),
            "icons/file.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/file.svg"))),
            "icons/copy.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/copy.svg"))),
            "icons/refresh.svg" => {
                Some(Cow::Borrowed(include_bytes!("../assets/icons/refresh.svg")))
            }
            "icons/undo.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/undo.svg"))),
            "icons/tag.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/tag.svg"))),
            "icons/trash.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/trash.svg"))),
            "icons/broom.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/broom.svg"))),
            "icons/infinity.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/infinity.svg"
            ))),
            "icons/arrow_left.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/arrow_left.svg"
            ))),
            "icons/arrow_right.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/arrow_right.svg"
            ))),
            "icons/link.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/link.svg"))),
            "icons/unlink.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/unlink.svg"))),
            "icons/cloud.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/cloud.svg"))),
            "icons/cog.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/cog.svg"))),
            "icons/computer.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/computer.svg"
            ))),
            "icons/folder.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/folder.svg"))),
            "icons/generic_minimize.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/generic_minimize.svg"
            ))),
            "icons/generic_maximize.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/generic_maximize.svg"
            ))),
            "icons/generic_restore.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/generic_restore.svg"
            ))),
            "icons/generic_close.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/generic_close.svg"
            ))),
            "icons/repo_tab_close.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/repo_tab_close.svg"
            ))),
            "icons/git_branch.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/git_branch.svg"
            ))),
            "icons/git_worktree.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/git_worktree.svg"
            ))),
            "icons/gitcomet_mark.svg" => Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/gitcomet_mark.svg"
            ))),
            "icons/menu.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/menu.svg"))),
            "icons/pencil.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/pencil.svg"))),
            _ => None,
        }
    }

    fn list_static(dir: &str) -> Vec<SharedString> {
        match dir.trim_end_matches('/') {
            "" => vec![
                "gitcomet-window-icon.png".into(),
                "gitcomet-512.png".into(),
                "gitcomet_logo.svg".into(),
                "icons".into(),
            ],
            "icons" => vec![
                "icons/arrow_down.svg".into(),
                "icons/arrow_up.svg".into(),
                "icons/spinner.svg".into(),
                "icons/box.svg".into(),
                "icons/check.svg".into(),
                "icons/chevron_down.svg".into(),
                "icons/plus.svg".into(),
                "icons/minus.svg".into(),
                "icons/question.svg".into(),
                "icons/warning.svg".into(),
                "icons/swap.svg".into(),
                "icons/open_external.svg".into(),
                "icons/file.svg".into(),
                "icons/copy.svg".into(),
                "icons/refresh.svg".into(),
                "icons/undo.svg".into(),
                "icons/tag.svg".into(),
                "icons/trash.svg".into(),
                "icons/broom.svg".into(),
                "icons/infinity.svg".into(),
                "icons/arrow_left.svg".into(),
                "icons/arrow_right.svg".into(),
                "icons/link.svg".into(),
                "icons/unlink.svg".into(),
                "icons/cloud.svg".into(),
                "icons/cog.svg".into(),
                "icons/computer.svg".into(),
                "icons/folder.svg".into(),
                "icons/generic_minimize.svg".into(),
                "icons/generic_maximize.svg".into(),
                "icons/generic_restore.svg".into(),
                "icons/generic_close.svg".into(),
                "icons/repo_tab_close.svg".into(),
                "icons/git_branch.svg".into(),
                "icons/git_worktree.svg".into(),
                "icons/gitcomet_mark.svg".into(),
                "icons/menu.svg".into(),
                "icons/pencil.svg".into(),
            ],
            _ => vec![],
        }
    }
}

impl AssetSource for GitCometAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::load_static(path))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::list_static(path))
    }
}
