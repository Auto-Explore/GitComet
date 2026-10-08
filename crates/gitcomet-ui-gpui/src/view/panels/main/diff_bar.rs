use super::*;

/// The file arrows' state: where the file sits and which way it can step.
pub(in crate::view) struct DiffBarNav {
    /// Zero-based place and count, when the list is known.
    pub(in crate::view) position: Option<(usize, usize)>,
    /// What the list is, for the count's tooltip.
    pub(in crate::view) context: Option<SharedString>,
    pub(in crate::view) can_prev: bool,
    pub(in crate::view) can_next: bool,
}

/// The bar's Stage button: which way it goes and how many files it takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct StageAction {
    pub(in crate::view) area: DiffArea,
    /// More than one when a status multi-selection wins, as it does for Space.
    pub(in crate::view) files: usize,
}

impl StageAction {
    /// "Stage", "Unstage", or "Stage (3)" for a selection, as the status
    /// sections count theirs.
    pub(in crate::view) fn label(self) -> String {
        let word = match self.area {
            DiffArea::Unstaged => "Stage",
            DiffArea::Staged => "Unstage",
        };
        match self.files {
            1 => word.to_string(),
            n => format!("{word} ({n})"),
        }
    }

    fn tooltip(self) -> String {
        let word = match self.area {
            DiffArea::Unstaged => "Stage",
            DiffArea::Staged => "Unstage",
        };
        match self.files {
            1 => format!("{word} this file"),
            n => format!("{word} the {n} selected files"),
        }
    }
}

/// "3 of 43 files".
pub(in crate::view) fn position_label(index: usize, count: usize) -> String {
    gitcomet_extension_api::DiffFilePosition::new(index, count).label()
}

impl MainPaneView {
    /// The file arrows' state for the open diff. `None` when the diff came
    /// from no list, so there is nothing to step through.
    pub(in crate::view) fn diff_bar_nav(
        &self,
        repo_id: Option<RepoId>,
        cx: &mut gpui::Context<Self>,
    ) -> Option<DiffBarNav> {
        let repo_id = repo_id?;
        if !self.store.policy.file_navigation || !show_diff_file_navigation(self.view_mode) {
            return None;
        }
        if self.store.binding.is_some() {
            // A hosted pane steps through its owner's list, and the owner
            // says where in it the file is.
            let options = &self.hosted_decor.as_ref()?.options;
            let navigation = &options.file_navigation;
            let (prev, next) = (navigation.previous.is_some(), navigation.next.is_some());
            let position = options
                .bar
                .position
                .filter(|position| position.index < position.count)
                .map(|position| (position.index, position.count));
            return (prev || next).then_some(DiffBarNav {
                position,
                context: None,
                can_prev: prev && position.is_none_or(|(index, _)| index > 0),
                can_next: next && position.is_none_or(|(index, count)| index + 1 < count),
            });
        }
        let place = self.diff_file_place(repo_id, cx)?;
        Some(DiffBarNav {
            position: Some((place.index, place.count)),
            context: Some(place.context),
            can_prev: place.index > 0,
            can_next: place.index + 1 < place.count,
        })
    }

    /// What the bar's Stage button would do, the same as Space: `None` unless
    /// the window's own pane shows a working-tree file outside the conflict
    /// resolver and staging is allowed.
    pub(in crate::view) fn stage_action(
        &self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<StageAction> {
        if self.store.binding.is_some()
            || !self.store.policy.allow_stage
            || !matches!(self.view_mode, GitCometViewMode::Normal)
            || self.is_inline_submodule_diff_active()
            || self.is_conflict_resolver_active()
            || self.is_file_editor_active()
        {
            return None;
        }
        let repo = self.active_repo()?;
        let DiffTarget::WorkingTree { path, area, .. } =
            self.bound_diff_state(repo).diff_target.as_ref()?
        else {
            return None;
        };
        let (area, path) = (*area, path.clone());
        let files = self
            .root_view
            .update(cx, |root, cx| {
                let (paths, used_selection) = root
                    .details_pane
                    .read(cx)
                    .status_selected_paths_for_action(repo_id, area, &path);
                used_selection.then_some(paths.len())
            })
            .ok()
            .flatten()
            .unwrap_or(1);
        Some(StageAction { area, files })
    }

    fn stage_button(
        &self,
        action: StageAction,
        theme: AppTheme,
        scale: crate::ui_scale::UiScale,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let pane = cx.entity().downgrade();
        let handle = window.window_handle();
        let icon = match action.area {
            DiffArea::Unstaged => "icons/plus.svg",
            DiffArea::Staged => "icons/minus.svg",
        };
        components::ActionButton::new("stage", action.label(), move |(), cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = pane.update(cx, |pane, cx| {
                    pane.toggle_stage_shown_file(window, cx);
                    window.focus(&pane.diff_panel_focus_handle, cx);
                    cx.notify();
                });
            });
        })
        .with_icon(icon)
        .with_shortcut("space")
        .with_tooltip(action.tooltip())
        .framed()
        .render("diff_bar_stage", (), theme, scale)
    }

    /// Uses the same action renderer as the built-in toolbar. Owner-built
    /// controls can extend the bar without adding another host API.
    fn hosted_bar_items(
        &self,
        theme: AppTheme,
        scale: crate::ui_scale::UiScale,
    ) -> Vec<AnyElement> {
        let Some(decor) = self.hosted_decor.as_ref() else {
            return Vec::new();
        };
        let selection = self.hosted_selection();
        decor
            .options
            .bar
            .items
            .iter()
            .map(|item| item.render(format!("diff_bar_{}", item.id), selection, theme, scale))
            .chain(
                decor
                    .options
                    .bar
                    .content
                    .iter()
                    .cloned()
                    .map(|view| view.into_any_element()),
            )
            .collect()
    }

    /// The diff's bottom bar: the file arrows and "3 of 43 files" on the
    /// left, format chips centred, and actions at the right. File-history
    /// controls sit beside file navigation. `None` when it would be empty.
    pub(super) fn diff_bar(
        &self,
        repo_id: Option<RepoId>,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let theme = self.theme;
        let scale = crate::ui_scale::UiScale::current(cx);
        let nav = self.diff_bar_nav(repo_id, cx);
        let chips = self.text_format_chips(cx);
        let viewer_nav = self.diff_viewer_nav_cluster(theme, cx);
        let stats = (self.diff_view == DiffViewMode::Split
            && self.is_collapsed_diff_projection_active())
        .then(|| self.collapsed_diff_total_file_stat())
        .flatten();
        let stage = repo_id.and_then(|repo_id| self.stage_action(repo_id, cx));
        let mut actions: Vec<AnyElement> = stage
            .map(|action| self.stage_button(action, theme, scale, window, cx))
            .into_iter()
            .collect();
        actions.extend(self.hosted_bar_items(theme, scale));
        if nav.is_none()
            && viewer_nav.is_none()
            && stats.is_none()
            && chips.is_none()
            && actions.is_empty()
        {
            return None;
        }

        let nav = nav.zip(repo_id).map(|(nav, repo_id)| {
            let arrow = |id: &'static str,
                         icon: &'static str,
                         label: &'static str,
                         keys: Vec<SharedString>,
                         delta: i8,
                         enabled: bool,
                         cx: &mut gpui::Context<Self>| {
                components::nav_arrow_button(id, icon, theme, scale, enabled)
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        if this.try_select_adjacent_diff_file(repo_id, delta, window, cx) {
                            cx.notify();
                        }
                    })
                    .debug_selector(move || id.to_string())
                    .gitcomet_tooltip_keyed(theme, label.into(), keys)
            };
            let position = nav.position.map(|(index, count)| {
                div()
                    .id("diff_bar_position")
                    .debug_selector(|| "diff_bar_position".to_string())
                    .pl(scale.px(4.0))
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .whitespace_nowrap()
                    .child(position_label(index, count))
                    .when_some(nav.context.clone(), |d, context| {
                        d.gitcomet_tooltip(theme, context)
                    })
            });
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(scale.px(2.0))
                .child(arrow(
                    "diff_prev_file",
                    "icons/arrow_left.svg",
                    "Previous file",
                    crate::view::shortcut_labels::previous_file_shortcuts(),
                    -1,
                    nav.can_prev,
                    cx,
                ))
                .child(arrow(
                    "diff_next_file",
                    "icons/arrow_right.svg",
                    "Next file",
                    crate::view::shortcut_labels::next_file_shortcuts(),
                    1,
                    nav.can_next,
                    cx,
                ))
                .children(position)
        });

        let stats = stats.map(|(added, removed)| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(scale.px(6.0))
                .text_size(theme.ui_text(12.0))
                .child(
                    div()
                        .debug_selector(|| "diff_bar_removed_stat".to_string())
                        .text_color(theme.colors.diff.removed.foreground)
                        .child(format!("-{removed}")),
                )
                .child(
                    div()
                        .debug_selector(|| "diff_bar_added_stat".to_string())
                        .text_color(theme.colors.diff.added.foreground)
                        .child(format!("+{added}")),
                )
        });
        // Equal-width side slots keep the format strip at the bar's centre
        // regardless of how much navigation or how many actions are present.
        Some(
            components::bottom_bar(theme, scale)
                .id("diff_bottom_bar")
                .debug_selector(|| "diff_bottom_bar".to_string())
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .flex_basis(px(0.0))
                        .min_w(px(0.0))
                        .items_center()
                        .overflow_hidden()
                        .gap(scale.px(6.0))
                        .children(nav)
                        .children(viewer_nav)
                        .children(stats),
                )
                .children(chips)
                .child(
                    div()
                        .id("diff_bar_actions")
                        .flex()
                        .flex_1()
                        .flex_basis(px(0.0))
                        .min_w(px(0.0))
                        .items_center()
                        .justify_end()
                        .overflow_hidden()
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(scale.px(4.0))
                                .children(actions),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_reads_as_a_place_among_files() {
        assert_eq!(position_label(0, 1), "1 of 1 file");
        assert_eq!(position_label(2, 43), "3 of 43 files");
    }

    #[test]
    fn the_stage_button_names_its_direction_and_a_selections_size() {
        let action = |area, files| StageAction { area, files }.label();
        assert_eq!(action(DiffArea::Unstaged, 1), "Stage");
        assert_eq!(action(DiffArea::Staged, 1), "Unstage");
        assert_eq!(action(DiffArea::Unstaged, 3), "Stage (3)");
    }
}
