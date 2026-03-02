use super::*;
use gitgpui_core::services::ConflictSide;

fn conflict_side_image(path: &std::path::Path, bytes: Option<&[u8]>) -> Option<Arc<gpui::Image>> {
    let format = crate::view::diff_utils::image_format_for_path(path)?;
    let bytes = bytes?;
    Some(Arc::new(gpui::Image::from_bytes(format, bytes.to_vec())))
}

impl MainPaneView {
    /// Render the binary/non-UTF8 conflict resolver panel.
    ///
    /// Shows file size info for each conflict side and provides "Use Base" /
    /// "Use Ours" / "Use Theirs" actions for binary-safe side checkout.
    pub(super) fn render_binary_conflict_resolver(
        &mut self,
        theme: AppTheme,
        repo_id: RepoId,
        path: std::path::PathBuf,
        file: &gitgpui_state::model::ConflictFile,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let [base_size, ours_size, theirs_size] = self.conflict_resolver.binary_side_sizes;

        let format_size = |size: Option<usize>| -> SharedString {
            match size {
                None => "absent".into(),
                Some(n) if n < 1024 => format!("{} B", n).into(),
                Some(n) if n < 1024 * 1024 => format!("{:.1} KiB", n as f64 / 1024.0).into(),
                Some(n) => format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0)).into(),
            }
        };

        let side_row = |label: &'static str, size: Option<usize>, has_text: bool| -> gpui::Div {
            let size_label = format_size(size);
            let kind_label: SharedString = if has_text {
                "text (valid UTF-8)".into()
            } else if size.is_some() {
                "binary (non-UTF8)".into()
            } else {
                "not present".into()
            };

            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.colors.text)
                        .w(px(80.0))
                        .child(label),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child(size_label),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if has_text {
                            theme.colors.text_muted
                        } else if size.is_some() {
                            theme.colors.warning
                        } else {
                            theme.colors.text_muted
                        })
                        .child(kind_label),
                )
        };

        let info_section = div()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .child(side_row("Base", base_size, file.base.is_some()))
            .child(side_row("Ours", ours_size, file.ours.is_some()))
            .child(side_row("Theirs", theirs_size, file.theirs.is_some()));

        let base_path = path.clone();
        let ours_path = path.clone();
        let theirs_path = path.clone();
        let mergetool_path = path.clone();

        let has_base = file.base_bytes.is_some();
        let has_ours = file.ours_bytes.is_some();
        let has_theirs = file.theirs_bytes.is_some();
        let has_image_preview = crate::view::diff_utils::image_format_for_path(&path).is_some();
        let base_image = conflict_side_image(&path, file.base_bytes.as_deref());
        let ours_image = conflict_side_image(&path, file.ours_bytes.as_deref());
        let theirs_image = conflict_side_image(&path, file.theirs_bytes.as_deref());

        let action_section = div()
            .flex()
            .items_center()
            .gap_2()
            .p_3()
            .child(
                zed::Button::new("binary_use_base", "Use Base (ancestor)")
                    .style(zed::ButtonStyle::Outlined)
                    .disabled(!has_base)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::CheckoutConflictBase {
                            repo_id,
                            path: base_path.clone(),
                        });
                    }),
            )
            .child(
                zed::Button::new("binary_use_ours", "Use Ours (local)")
                    .style(zed::ButtonStyle::Outlined)
                    .disabled(!has_ours)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::CheckoutConflictSide {
                            repo_id,
                            path: ours_path.clone(),
                            side: ConflictSide::Ours,
                        });
                    }),
            )
            .child(
                zed::Button::new("binary_use_theirs", "Use Theirs (remote)")
                    .style(zed::ButtonStyle::Outlined)
                    .disabled(!has_theirs)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::CheckoutConflictSide {
                            repo_id,
                            path: theirs_path.clone(),
                            side: ConflictSide::Theirs,
                        });
                    }),
            )
            .child(div().w(px(1.0)).h(px(16.0)).bg(theme.colors.border))
            .child(
                zed::Button::new("binary_launch_mergetool", "External Mergetool")
                    .style(zed::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::LaunchMergetool {
                            repo_id,
                            path: mergetool_path.clone(),
                        });
                    }),
            );

        let image_preview = has_image_preview.then(|| {
            let image_cell =
                |id: &'static str, label: &'static str, image: Option<Arc<gpui::Image>>| {
                    div()
                        .id(id)
                        .flex_1()
                        .min_w(px(0.0))
                        .h_full()
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(px(theme.radii.row))
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .h(px(24.0))
                                .px_2()
                                .flex()
                                .items_center()
                                .justify_between()
                                .bg(theme.colors.surface_bg_elevated)
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(0.0))
                                .bg(theme.colors.window_bg)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(match image {
                                    Some(img_data) => gpui::img(img_data)
                                        .w_full()
                                        .h_full()
                                        .object_fit(gpui::ObjectFit::Contain)
                                        .into_any_element(),
                                    None => div()
                                        .text_xs()
                                        .text_color(theme.colors.text_muted)
                                        .child("No image")
                                        .into_any_element(),
                                }),
                        )
                };

            div()
                .w_full()
                .px_3()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.colors.text)
                        .child("Image preview"),
                )
                .child(
                    div()
                        .h(px(180.0))
                        .w_full()
                        .mt_2()
                        .flex()
                        .gap_2()
                        .child(image_cell(
                            "binary_conflict_preview_base",
                            "Base (A)",
                            base_image,
                        ))
                        .child(image_cell(
                            "binary_conflict_preview_ours",
                            "Ours (B)",
                            ours_image,
                        ))
                        .child(image_cell(
                            "binary_conflict_preview_theirs",
                            "Theirs (C)",
                            theirs_image,
                        )),
                )
        });

        let title: SharedString =
            format!("Resolve conflict: {}", self.cached_path_display(&path)).into();

        div()
            .id("binary_conflict_resolver_panel")
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .px_2()
            .py_2()
            .gap_2()
            // Header
            .child(
                div().flex().items_center().gap_2().child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.colors.text)
                        .child(title),
                ),
            )
            // Content panel
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(px(theme.radii.row))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .bg(theme.colors.window_bg)
                    // Icon/label
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.colors.warning)
                            .child("Binary file conflict"),
                    )
                    .child(div().text_sm().text_color(theme.colors.text_muted).child(
                        "This file contains binary or non-UTF8 data and cannot be merged as text.",
                    ))
                    .when_some(image_preview, |d, preview| d.child(preview))
                    // Side info
                    .child(
                        div()
                            .border_1()
                            .border_color(theme.colors.border)
                            .rounded(px(theme.radii.row))
                            .bg(theme.colors.surface_bg_elevated)
                            .child(info_section),
                    )
                    // Action buttons
                    .child(action_section),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::conflict_side_image;

    #[test]
    fn conflict_side_image_requires_supported_path_extension() {
        let bytes = [1_u8, 2, 3, 4];
        assert!(conflict_side_image("file.bin".as_ref(), Some(&bytes)).is_none());
    }

    #[test]
    fn conflict_side_image_requires_bytes() {
        assert!(conflict_side_image("file.png".as_ref(), None).is_none());
    }

    #[test]
    fn conflict_side_image_builds_for_image_path_and_bytes() {
        let bytes = [1_u8, 2, 3, 4];
        assert!(conflict_side_image("file.png".as_ref(), Some(&bytes)).is_some());
    }
}
