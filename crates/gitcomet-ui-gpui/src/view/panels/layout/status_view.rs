//! The details pane with nothing selected: the status sections with their
//! actions and resize handles, and the commit box.

use super::*;

/// One status section's numbers, as its header and body read them.
#[derive(Clone, Copy)]
struct StatusSectionState {
    count: usize,
    selected: usize,
    loading: bool,
    labels: StatusActionLabels,
}

/// What every piece of the status view reads, gathered once per render.
struct StatusViewInputs {
    theme: AppTheme,
    ui_scale: ui_scale::UiScale,
    ui_scale_percent: u32,
    repo_id: Option<RepoId>,
    repo_key: u64,
    local_actions_in_flight: bool,
    split_change_tracking: bool,
    icon_muted: gpui::Rgba,
    unstaged: StatusSectionState,
    untracked: StatusSectionState,
    split_unstaged: StatusSectionState,
    staged: StatusSectionState,
}

impl StatusViewInputs {
    fn state(&self, section: StatusSection) -> StatusSectionState {
        match section {
            StatusSection::CombinedUnstaged => self.unstaged,
            StatusSection::Untracked => self.untracked,
            StatusSection::Unstaged => self.split_unstaged,
            StatusSection::Staged => self.staged,
        }
    }
}

/// A split's (top, bottom) heights; `None` until the container is measured.
type SplitHeights = Option<(Pixels, Pixels)>;

/// The fixed ids and wording of one status section's pieces.
#[derive(Clone, Copy)]
struct StatusSectionNames {
    section: StatusSection,
    header_id: &'static str,
    controls_id_prefix: &'static str,
    spinner_id: &'static str,
    empty_message: &'static str,
}

const COMBINED_UNSTAGED_NAMES: StatusSectionNames = StatusSectionNames {
    section: StatusSection::CombinedUnstaged,
    header_id: "unstaged_header",
    controls_id_prefix: "status_unstaged",
    spinner_id: "unstaged_actions_spinner",
    empty_message: "No unstaged changes.",
};

const UNTRACKED_NAMES: StatusSectionNames = StatusSectionNames {
    section: StatusSection::Untracked,
    header_id: "untracked_header",
    controls_id_prefix: "status_untracked",
    spinner_id: "untracked_actions_spinner",
    empty_message: "No untracked files.",
};

const SPLIT_UNSTAGED_NAMES: StatusSectionNames = StatusSectionNames {
    section: StatusSection::Unstaged,
    header_id: "split_unstaged_header",
    controls_id_prefix: "status_split_unstaged",
    spinner_id: "split_unstaged_actions_spinner",
    empty_message: "No unstaged changes.",
};

const STAGED_NAMES: StatusSectionNames = StatusSectionNames {
    section: StatusSection::Staged,
    header_id: "staged_header",
    controls_id_prefix: "status_staged",
    spinner_id: "staged_actions_spinner",
    empty_message: "Nothing staged yet.",
};

impl DetailsPaneView {
    pub(super) fn status_sections_view(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let v = self.status_view_inputs(cx);
        let status_sections = self.status_sections_column(&v, cx);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .h_full()
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.finish_status_section_resize(cx);
                }),
            )
            .child(if v.repo_id.is_some() {
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(status_sections)
                    .child(div().px_2().py_2().child(self.commit_box(cx)))
                    .into_any_element()
            } else {
                components::empty_state(v.theme, "Changes", "No repository selected.")
                    .into_any_element()
            })
            .into_any_element()
    }

    /// The paths a section lists, in its display order.
    fn status_section_paths(&self, section: StatusSection) -> Vec<std::path::PathBuf> {
        self.active_repo()
            .and_then(|repo| self.status_section_entries(repo, section))
            .map_or_else(Vec::new, |entries| entries.path_vec())
    }

    fn status_view_inputs(&self, cx: &mut gpui::Context<Self>) -> StatusViewInputs {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let local_actions_in_flight = self
            .active_repo()
            .map(|r| r.local_actions_in_flight > 0)
            .unwrap_or(false);
        let (staged_count, unstaged_count, untracked_count, split_unstaged_count) = self
            .active_repo()
            .map(|repo| {
                (
                    self.status_section_entries(repo, StatusSection::Staged)
                        .map_or(0, |entries| entries.len()),
                    self.status_section_entries(repo, StatusSection::CombinedUnstaged)
                        .map_or(0, |entries| entries.len()),
                    self.status_section_entries(repo, StatusSection::Untracked)
                        .map_or(0, |entries| entries.len()),
                    self.status_section_entries(repo, StatusSection::Unstaged)
                        .map_or(0, |entries| entries.len()),
                )
            })
            .unwrap_or((0, 0, 0, 0));
        let (unstaged_loading, untracked_loading, split_unstaged_loading, staged_loading) = self
            .active_repo()
            .map(|repo| {
                (
                    status_section_is_loading(repo, StatusSection::CombinedUnstaged),
                    status_section_is_loading(repo, StatusSection::Untracked),
                    status_section_is_loading(repo, StatusSection::Unstaged),
                    status_section_is_loading(repo, StatusSection::Staged),
                )
            })
            .unwrap_or((false, false, false, false));

        let repo_id = self.active_repo_id();
        let selected_combined_unstaged = repo_id
            .map(|rid| self.status_section_action_count(rid, StatusSection::CombinedUnstaged))
            .unwrap_or(0);
        let selected_untracked = repo_id
            .map(|rid| self.status_section_action_count(rid, StatusSection::Untracked))
            .unwrap_or(0);
        let selected_split_unstaged = repo_id
            .map(|rid| self.status_section_action_count(rid, StatusSection::Unstaged))
            .unwrap_or(0);
        let selected_staged = repo_id
            .map(|rid| self.status_section_action_count(rid, StatusSection::Staged))
            .unwrap_or(0);

        let repo_key = repo_id.map(|id| id.0).unwrap_or(0);
        let split_change_tracking = self.change_tracking_view == ChangeTrackingView::SplitUntracked;
        let icon_muted = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.72 } else { 0.82 },
        );
        let ui_scale_percent = crate::ui_scale::current(cx).percent;

        // Measured last frame by the probe on the sections container; the
        // prepaint callback refreshes the window when it changes. Unmeasured on
        // the very first frame, which reads as "plenty of room" and settles on
        // the next one.
        let header_width = self
            .current_status_sections_bounds()
            .map(|bounds| bounds.size.width)
            .unwrap_or(Pixels::MAX);
        let labels_for =
            |title_chars: usize, title_is_dropdown: bool, action_label_chars: &[usize]| {
                status_action_labels_for_width(
                    header_width,
                    title_chars,
                    title_is_dropdown,
                    action_label_chars,
                    local_actions_in_flight,
                    ui_scale_percent,
                    theme.metrics,
                )
            };
        let count_chars =
            |word: &str, count: usize| word.chars().count() + 3 + count.to_string().len();
        let unstaged_labels = if selected_combined_unstaged > 0 {
            labels_for(
                "Unstaged".len(),
                true,
                &[
                    count_chars("Stage", selected_combined_unstaged),
                    count_chars("Discard", selected_combined_unstaged),
                    "Stage all changes".len(),
                ],
            )
        } else {
            labels_for("Unstaged".len(), true, &["Stage all changes".len()])
        };
        let untracked_labels = if selected_untracked > 0 {
            labels_for(
                "Untracked".len(),
                true,
                &[
                    count_chars("Stage", selected_untracked),
                    count_chars("Discard", selected_untracked),
                    "Stage all".len(),
                ],
            )
        } else {
            labels_for("Untracked".len(), true, &["Stage all".len()])
        };
        let split_unstaged_labels = if selected_split_unstaged > 0 {
            labels_for(
                "Unstaged".len(),
                true,
                &[
                    count_chars("Stage", selected_split_unstaged),
                    count_chars("Discard", selected_split_unstaged),
                    "Stage all".len(),
                ],
            )
        } else {
            labels_for("Unstaged".len(), true, &["Stage all".len()])
        };
        let staged_labels = if selected_staged > 0 {
            labels_for(
                "Staged".len(),
                false,
                &[
                    count_chars("Unstage", selected_staged),
                    "Unstage all changes".len(),
                ],
            )
        } else {
            labels_for("Staged".len(), false, &["Unstage all changes".len()])
        };

        StatusViewInputs {
            theme,
            ui_scale,
            ui_scale_percent,
            repo_id,
            repo_key,
            local_actions_in_flight,
            split_change_tracking,
            icon_muted,
            unstaged: StatusSectionState {
                count: unstaged_count,
                selected: selected_combined_unstaged,
                loading: unstaged_loading,
                labels: unstaged_labels,
            },
            untracked: StatusSectionState {
                count: untracked_count,
                selected: selected_untracked,
                loading: untracked_loading,
                labels: untracked_labels,
            },
            split_unstaged: StatusSectionState {
                count: split_unstaged_count,
                selected: selected_split_unstaged,
                loading: split_unstaged_loading,
                labels: split_unstaged_labels,
            },
            staged: StatusSectionState {
                count: staged_count,
                selected: selected_staged,
                loading: staged_loading,
                labels: staged_labels,
            },
        }
    }

    /// The combined view's "Unstaged" section: tracked and untracked changes.
    fn combined_unstaged_status_section(
        &mut self,
        v: &StatusViewInputs,
        cx: &mut gpui::Context<Self>,
    ) -> Div {
        let names = COMBINED_UNSTAGED_NAMES;
        let stage_all = Self::stage_all_button(v, cx);
        let stage_selected = Self::stage_selected_button(
            v,
            "stage_selected",
            Some("stage_selected_button"),
            names.section,
            cx,
        );
        let discard_selected =
            Self::discard_selected_button(v, "discard_selected", names.section, cx);
        let actions = self.status_section_actions(
            v,
            names,
            [stage_selected, discard_selected],
            stage_all,
            cx,
        );
        let body = self.status_section_body(v, names, cx);
        let title = self.change_tracking_header_title(
            v,
            "change_tracking_unstaged_header",
            "change_tracking_unstaged_header",
            "Unstaged",
            cx,
        );
        self.status_section_frame(v, names, title, actions, body, cx)
    }

    /// The split view's "Untracked" section.
    fn untracked_status_section(
        &mut self,
        v: &StatusViewInputs,
        cx: &mut gpui::Context<Self>,
    ) -> Div {
        let names = UNTRACKED_NAMES;
        let stage_all = Self::stage_all_untracked_button(v, cx);
        let stage_selected =
            Self::stage_selected_button(v, "stage_selected_untracked", None, names.section, cx);
        let discard_selected =
            Self::discard_selected_button(v, "discard_selected_untracked", names.section, cx);
        let actions = self.status_section_actions(
            v,
            names,
            [stage_selected, discard_selected],
            stage_all,
            cx,
        );
        let body = self.status_section_body(v, names, cx);
        let title = self.change_tracking_header_title(
            v,
            "change_tracking_untracked_header",
            "change_tracking_untracked_header",
            "Untracked",
            cx,
        );
        self.status_section_frame(v, names, title, actions, body, cx)
    }

    /// The split view's "Unstaged" section: tracked changes only.
    fn split_unstaged_status_section(
        &mut self,
        v: &StatusViewInputs,
        cx: &mut gpui::Context<Self>,
    ) -> Div {
        let names = SPLIT_UNSTAGED_NAMES;
        let stage_all = Self::stage_all_split_unstaged_button(v, cx);
        let stage_selected = Self::stage_selected_button(
            v,
            "stage_selected_split_unstaged",
            None,
            names.section,
            cx,
        );
        let discard_selected =
            Self::discard_selected_button(v, "discard_selected_split_unstaged", names.section, cx);
        let actions = self.status_section_actions(
            v,
            names,
            [stage_selected, discard_selected],
            stage_all,
            cx,
        );
        let body = self.status_section_body(v, names, cx);
        let title = self.change_tracking_header_title(
            v,
            "change_tracking_unstaged_header",
            "change_tracking_unstaged_header",
            "Unstaged",
            cx,
        );
        self.status_section_frame(v, names, title, actions, body, cx)
    }

    fn staged_status_section(&mut self, v: &StatusViewInputs, cx: &mut gpui::Context<Self>) -> Div {
        let names = STAGED_NAMES;
        let unstage_all = Self::unstage_all_button(v, cx);
        let unstage_selected = Self::unstage_selected_button(v, cx);
        let actions = self.status_section_actions(v, names, [unstage_selected], unstage_all, cx);
        let body = self.status_section_body(v, names, cx);
        let title = status_section_title(v.theme, "Staged");
        self.status_section_frame(v, names, title, actions, body, cx)
    }

    /// A header's action group: list controls, the busy spinner, the buttons
    /// for the current selection (only while there is one), then the
    /// section-wide button.
    fn status_section_actions<const N: usize>(
        &mut self,
        v: &StatusViewInputs,
        names: StatusSectionNames,
        selection_buttons: [Stateful<Div>; N],
        all_button: Stateful<Div>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let controls = v.repo_id.map(|repo_id| {
            self.file_list_controls(
                crate::view::rows::FileListId::Status(names.section),
                repo_id,
                names.controls_id_prefix,
                false,
                cx,
            )
        });
        let mut actions = div().flex().items_center().gap_2();
        if let Some(controls) = controls {
            actions = actions.child(controls);
        }
        if v.local_actions_in_flight {
            actions = actions.child(
                svg_spinner(
                    (names.spinner_id, v.repo_key),
                    with_alpha(
                        v.theme.colors.accent.foreground,
                        if v.theme.is_dark { 0.72 } else { 0.82 },
                    ),
                    v.ui_scale.px(14.0),
                )
                .into_any_element(),
            );
        }
        if v.state(names.section).selected > 0 {
            for button in selection_buttons {
                actions = actions.child(button);
            }
        }
        actions.child(all_button).into_any_element()
    }

    /// A section's file list, or its loading or empty message.
    fn status_section_body(
        &mut self,
        v: &StatusViewInputs,
        names: StatusSectionNames,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let state = v.state(names.section);
        if state.loading {
            components::empty_state_message(v.theme, "Loading…").into_any_element()
        } else if state.count == 0 {
            components::empty_state_message(v.theme, names.empty_message).into_any_element()
        } else {
            self.status_list(cx, names.section, state.count)
        }
    }

    /// The section's focusable container: header on top, list filling the rest.
    fn status_section_frame(
        &self,
        v: &StatusViewInputs,
        names: StatusSectionNames,
        title: AnyElement,
        actions: AnyElement,
        body: AnyElement,
        cx: &gpui::Context<Self>,
    ) -> Div {
        self.status_section_container(names.section, cx)
            .flex()
            .flex_col()
            .min_h(px(STATUS_SECTION_MIN_HEIGHT_PX))
            .overflow_hidden()
            .child(status_section_header(
                v,
                names.header_id,
                title,
                v.state(names.section).count > 0,
                actions,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(body),
            )
    }

    /// The "Unstaged" / "Untracked" title chip that opens the change-tracking
    /// settings.
    fn change_tracking_header_title(
        &self,
        v: &StatusViewInputs,
        id: &'static str,
        invoker_key: &'static str,
        label: &'static str,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let theme = v.theme;
        let ui_scale = v.ui_scale;
        let change_tracking_invoker: SharedString = invoker_key.into();
        let change_tracking_active =
            self.active_context_menu_invoker.as_ref() == Some(&change_tracking_invoker);
        let change_tracking_invoker = change_tracking_invoker.clone();
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .h(ui_scale.row_height(
                CHANGE_TRACKING_HEADER_CHIP_HEIGHT_PX,
                CHANGE_TRACKING_HEADER_CHIP_COMFORTABLE_HEIGHT_PX,
            ))
            .rounded(px(theme.radii.row))
            .tab_index(0)
            .control_interaction(
                InteractionStyle::header(theme),
                InteractionState::default().open(change_tracking_active),
            )
            .child(
                div()
                    .text_size(theme.ui_text(14.0))
                    .font_weight(FontWeight::BOLD)
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .child(label),
            )
            .child(
                svg_icon("icons/chevron_down.svg", v.icon_muted, ui_scale.px(12.0))
                    .debug_selector(move || format!("{id}_chevron")),
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(move |this, e: &ClickEvent, window, cx| {
                    this.open_popover_at(
                        PopoverKind::ChangeTrackingSettings
                            .invoked_by(change_tracking_invoker.clone()),
                        e.position(),
                        window,
                        cx,
                    );
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    // Action buttons. Every one is built each frame, shown or not.

    fn stage_all_button(v: &StatusViewInputs, cx: &mut gpui::Context<Self>) -> Stateful<Div> {
        let theme = v.theme;
        components::Button::new(
            "stage_all",
            status_action_all_label(v.unstaged.labels, "Stage all changes"),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(v.local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            // Empty paths: this button stages every change there is.
            this.stage_all_with_conflict_confirmation(repo_id, Vec::new(), _w, cx);
        })
        .debug_selector(|| "stage_all_button".to_string())
        .gitcomet_tooltip(theme, "Stage all changes".into())
    }

    /// Stages the section's selection; shared by the three unstaged sections.
    fn stage_selected_button(
        v: &StatusViewInputs,
        id: &'static str,
        debug_selector: Option<&'static str>,
        section: StatusSection,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<Div> {
        let theme = v.theme;
        let state = v.state(section);
        let selected = state.selected;
        let button = components::Button::new(
            id,
            status_action_count_label(state.labels, "Stage", selected),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(v.local_actions_in_flight)
        .on_click(theme, cx, move |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            // Read without consuming: the confirmation below can still be
            // cancelled, and that must leave the selection as the user built it.
            let selection = this.status_section_action_selection(repo_id, section);
            let paths = selection.paths;
            if paths.is_empty() {
                return;
            }
            if let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
                &this.state,
                repo_id,
                paths.clone(),
                selection.from_explicit_selection,
            ) {
                let anchor = crate::view::conflict_markers::centered_dialog_anchor(_w);
                this.open_popover_at(confirm, anchor, _w, cx);
                cx.notify();
                return;
            }
            if selection.from_explicit_selection {
                this.clear_status_multi_selection(repo_id);
            }
            crate::view::status_actions::stage_or_unstage_paths(
                &this.store,
                repo_id,
                DiffArea::Unstaged,
                paths,
            );
            cx.notify();
        });
        let button = match debug_selector {
            Some(selector) => button.debug_selector(move || selector.to_string()),
            None => button,
        };
        button.gitcomet_tooltip(
            theme,
            format!(
                "Stage {selected} selected {}",
                status_action_file_count(selected)
            )
            .into(),
        )
    }

    /// Asks to discard the section's selection; shared by the three unstaged
    /// sections.
    fn discard_selected_button(
        v: &StatusViewInputs,
        id: &'static str,
        section: StatusSection,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<Div> {
        let theme = v.theme;
        let state = v.state(section);
        let selected = state.selected;
        components::Button::new(
            id,
            status_action_count_label(state.labels, "Discard", selected),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(v.local_actions_in_flight)
        .on_click(theme, cx, move |this, e, window, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let selection = this.status_section_action_selection(repo_id, section);
            if selection.paths.is_empty() {
                return;
            }
            this.open_popover_at(
                PopoverKind::DiscardChangesConfirm {
                    repo_id,
                    area: DiffArea::Unstaged,
                    path: selection.popover_path(),
                },
                e.position(),
                window,
                cx,
            );
            cx.notify();
        })
        .gitcomet_tooltip(
            theme,
            format!(
                "Discard changes in {selected} selected {}",
                status_action_file_count(selected)
            )
            .into(),
        )
    }

    fn stage_all_untracked_button(
        v: &StatusViewInputs,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<Div> {
        let theme = v.theme;
        components::Button::new(
            "stage_all_untracked",
            status_action_all_label(v.untracked.labels, "Stage all"),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(v.local_actions_in_flight || v.untracked.count == 0)
        .on_click(theme, cx, move |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            // Collected on click: copying the section's paths on every render
            // cost one allocation per file.
            let paths = this.status_section_paths(StatusSection::Untracked);
            if paths.is_empty() {
                return;
            }
            this.status_multi_selection.remove(&repo_id);
            crate::view::status_actions::stage_or_unstage_paths(
                &this.store,
                repo_id,
                DiffArea::Unstaged,
                gitcomet_state::msg::RepoPathList::from(paths),
            );
            cx.notify();
        })
        .debug_selector(|| "stage_all_untracked_button".to_string())
        .gitcomet_tooltip(theme, "Stage all untracked files".into())
    }

    fn stage_all_split_unstaged_button(
        v: &StatusViewInputs,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<Div> {
        let theme = v.theme;
        components::Button::new(
            "stage_all_split_unstaged",
            status_action_all_label(v.split_unstaged.labels, "Stage all"),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(v.local_actions_in_flight || v.split_unstaged.count == 0)
        .on_click(theme, cx, move |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let split_unstaged_paths_for_stage_all =
                this.status_section_paths(StatusSection::Unstaged);
            if split_unstaged_paths_for_stage_all.is_empty() {
                return;
            }
            // Named paths: this button stages the tracked-changes section only —
            // conflicted files among them, so it needs the same confirmation the
            // combined view's button gets.
            this.stage_all_with_conflict_confirmation(
                repo_id,
                split_unstaged_paths_for_stage_all,
                _w,
                cx,
            );
        })
        .debug_selector(|| "stage_all_split_unstaged_button".to_string())
        .gitcomet_tooltip(theme, "Stage all unstaged changes".into())
    }

    fn unstage_all_button(v: &StatusViewInputs, cx: &mut gpui::Context<Self>) -> Stateful<Div> {
        let theme = v.theme;
        components::Button::new(
            "unstage_all",
            status_action_all_label(v.staged.labels, "Unstage all changes"),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(v.local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            this.status_multi_selection.remove(&repo_id);
            crate::view::status_actions::stage_or_unstage_paths(
                &this.store,
                repo_id,
                DiffArea::Staged,
                gitcomet_state::msg::RepoPathList::default(),
            );
            cx.notify();
        })
        .debug_selector(|| "unstage_all_button".to_string())
        .gitcomet_tooltip(theme, "Unstage all changes".into())
    }

    fn unstage_selected_button(
        v: &StatusViewInputs,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<Div> {
        let theme = v.theme;
        let selected_staged = v.staged.selected;
        components::Button::new(
            "unstage_selected",
            status_action_count_label(v.staged.labels, "Unstage", selected_staged),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(v.local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let paths = this
                .take_status_section_action_selection(repo_id, StatusSection::Staged)
                .paths;
            if paths.is_empty() {
                return;
            }
            crate::view::status_actions::stage_or_unstage_paths(
                &this.store,
                repo_id,
                DiffArea::Staged,
                paths,
            );
            cx.notify();
        })
        .debug_selector(|| "unstage_selected_button".to_string())
        .gitcomet_tooltip(
            theme,
            format!(
                "Unstage {selected_staged} selected {}",
                status_action_file_count(selected_staged)
            )
            .into(),
        )
    }

    // Layout: split heights, resize handles, and the sections column.

    /// The (change tracking, staged) and (untracked, split unstaged) splits,
    /// from last frame's measurements.
    fn status_section_split_heights(
        &self,
        split_change_tracking: bool,
    ) -> (SplitHeights, SplitHeights) {
        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let resize_handle_h = px(PANE_RESIZE_HANDLE_PX);
        let change_tracking_total_height =
            self.measured_status_sections_total_height(resize_handle_h);
        let change_tracking_heights = change_tracking_total_height.map(|total_height| {
            let top_height = resolved_vertical_split_height(
                self.change_tracking_height,
                total_height,
                min_change_tracking_stack_height(split_change_tracking, resize_handle_h),
                section_min_h,
            );
            (top_height, (total_height - top_height).max(section_min_h))
        });

        let untracked_total_height =
            self.resolved_measured_change_tracking_stack_total_height(resize_handle_h);
        let untracked_heights = untracked_total_height.map(|total_height| {
            let top_height = resolved_vertical_split_height(
                self.untracked_height,
                total_height,
                section_min_h,
                section_min_h,
            );
            (top_height, (total_height - top_height).max(section_min_h))
        });
        (change_tracking_heights, untracked_heights)
    }

    fn status_resize_handle(
        &self,
        v: &StatusViewInputs,
        id: &'static str,
        handle: StatusSectionResizeHandle,
        cx: &gpui::Context<Self>,
    ) -> Stateful<Div> {
        let theme = v.theme;
        let dragging = self
            .status_section_resize
            .is_some_and(|state| state.handle == handle);
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .group(id)
            .w_full()
            .h(px(PANE_RESIZE_HANDLE_PX))
            .flex_none()
            .cursor(CursorStyle::ResizeUpDown)
            .child(components::resize_grip(
                theme,
                v.ui_scale,
                id,
                components::ResizeGripAxis::Horizontal,
                dragging,
                Some(theme.colors.stroke.default),
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    crate::press_gesture::claim_press(cx);
                    crate::text_selection_owner::preserve(cx);
                    this.start_status_section_resize(handle, e.position.y, cx);
                    window.refresh();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _e, window, cx| {
                    if this
                        .status_section_resize
                        .is_some_and(|state| state.handle == handle)
                    {
                        this.finish_status_section_resize(cx);
                        window.refresh();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |this, _e, window, cx| {
                    if this
                        .status_section_resize
                        .is_some_and(|state| state.handle == handle)
                    {
                        this.finish_status_section_resize(cx);
                        window.refresh();
                    }
                }),
            )
    }

    /// The split view's change-tracking stack: untracked over tracked, with
    /// its own resize handle and bounds probe.
    fn split_change_tracking_stack(
        &self,
        v: &StatusViewInputs,
        untracked_section: Div,
        split_unstaged_section: Div,
        untracked_heights: SplitHeights,
        cx: &gpui::Context<Self>,
    ) -> Div {
        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let resize_handle_h = px(PANE_RESIZE_HANDLE_PX);
        let change_tracking_stack_bounds_for_prepaint =
            std::rc::Rc::clone(&self.change_tracking_stack_bounds_ref);
        let stack_container = div()
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .min_w_full()
            .max_w_full()
            .h_full()
            .min_h(min_change_tracking_stack_height(
                v.split_change_tracking,
                resize_handle_h,
            ))
            .overflow_hidden()
            .on_children_prepainted(move |children_bounds, window, _app| {
                let next_bounds = children_bounds.first().copied();
                let mut measured = change_tracking_stack_bounds_for_prepaint.borrow_mut();
                if *measured != next_bounds {
                    *measured = next_bounds;
                    window.refresh();
                }
            });
        let untracked_top_height = untracked_heights.map(|(top_height, _)| top_height);
        let split_unstaged_height = untracked_heights.map(|(_, bottom_height)| bottom_height);
        let (untracked_grow, split_unstaged_grow) = untracked_heights
            .map(|(top_height, bottom_height)| (px_to_grow(top_height), px_to_grow(bottom_height)))
            .unwrap_or((1.0, 1.0));
        stack_container
            .child(visible_bounds_probe())
            .child(
                with_split_sizing(
                    untracked_section,
                    untracked_top_height,
                    untracked_grow,
                    section_min_h,
                )
                .debug_selector(|| "status_untracked_wrapper".to_string()),
            )
            .child(self.status_resize_handle(
                v,
                "status_resize_untracked_unstaged",
                StatusSectionResizeHandle::UntrackedAndUnstaged,
                cx,
            ))
            .child(
                with_split_sizing(
                    split_unstaged_section,
                    split_unstaged_height,
                    split_unstaged_grow,
                    section_min_h,
                )
                .debug_selector(|| "status_split_unstaged_wrapper".to_string()),
            )
    }

    /// Change tracking over staged, split by a resize handle. The container's
    /// probe measures it for next frame's split heights and header wording.
    fn status_sections_column(
        &mut self,
        v: &StatusViewInputs,
        cx: &mut gpui::Context<Self>,
    ) -> Div {
        let staged_section = self.staged_status_section(v, cx);
        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let resize_handle_h = px(PANE_RESIZE_HANDLE_PX);
        let (change_tracking_heights, untracked_heights) =
            self.status_section_split_heights(v.split_change_tracking);
        let change_tracking_section = if v.split_change_tracking {
            let untracked_section = self.untracked_status_section(v, cx);
            let split_unstaged_section = self.split_unstaged_status_section(v, cx);
            self.split_change_tracking_stack(
                v,
                untracked_section,
                split_unstaged_section,
                untracked_heights,
                cx,
            )
        } else {
            self.combined_unstaged_status_section(v, cx)
        };
        let (change_tracking_grow, staged_grow) = change_tracking_heights
            .map(|(top_height, bottom_height)| (px_to_grow(top_height), px_to_grow(bottom_height)))
            .unwrap_or((1.0, 1.0));
        let change_tracking_section = with_split_sizing(
            change_tracking_section,
            change_tracking_heights.map(|(top_height, _)| top_height),
            change_tracking_grow,
            min_change_tracking_stack_height(v.split_change_tracking, resize_handle_h),
        );
        let staged_section = with_split_sizing(
            staged_section,
            change_tracking_heights.map(|(_, bottom_height)| bottom_height),
            staged_grow,
            section_min_h,
        );
        let change_tracking_section =
            change_tracking_section.debug_selector(|| "status_change_tracking_wrapper".to_string());
        let staged_section = staged_section.debug_selector(|| "status_staged_wrapper".to_string());
        let status_sections_bounds_for_prepaint =
            std::rc::Rc::clone(&self.status_sections_bounds_ref);
        let status_sections_container = div()
            .relative()
            .w_full()
            .min_w_full()
            .max_w_full()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .on_children_prepainted(move |children_bounds, window, _app| {
                let next_bounds = children_bounds.first().copied();
                let mut measured = status_sections_bounds_for_prepaint.borrow_mut();
                if *measured != next_bounds {
                    *measured = next_bounds;
                    window.refresh();
                }
            });
        status_sections_container
            .child(visible_bounds_probe())
            .flex()
            .flex_col()
            .child(change_tracking_section)
            .child(self.status_resize_handle(
                v,
                "status_resize_change_tracking_staged",
                StatusSectionResizeHandle::ChangeTrackingAndStaged,
                cx,
            ))
            .child(staged_section)
    }
}

/// A section header: the title takes the slack and clips, the actions keep
/// their width.
fn status_section_header(
    v: &StatusViewInputs,
    id: &'static str,
    title: AnyElement,
    show_action: bool,
    action: AnyElement,
) -> AnyElement {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .h(components::content_header_height(
            ui_scale::UiScale::from_percent(v.ui_scale_percent).with_appearance(v.theme.metrics),
        ))
        .px_2()
        .overflow_hidden()
        // The labels shrink before this matters, but a UI zoom or a font
        // wider than the budget assumes can still overrun the header —
        // and then the title, not the actions, is what gives way.
        .child(
            div()
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(title),
        )
        .when(show_action, |d| d.child(div().flex_none().child(action)))
        .into_any_element()
}

/// A plain bold section title (the staged section's).
fn status_section_title(theme: AppTheme, label: &'static str) -> AnyElement {
    div()
        .text_size(theme.ui_text(14.0))
        .font_weight(FontWeight::BOLD)
        .line_clamp(1)
        .whitespace_nowrap()
        .child(label)
        .into_any_element()
}

/// Pins a section to `exact_height` once measured; until then it grows by
/// `fallback_grow` against its sibling.
fn with_split_sizing(
    mut section: Div,
    exact_height: Option<Pixels>,
    fallback_grow: f32,
    min_h: Pixels,
) -> Div {
    section = section.min_h(min_h);
    if let Some(exact_height) = exact_height {
        let exact_height = exact_height.max(min_h);
        section = section.h(exact_height).max_h(exact_height);
        section.style().flex_grow = Some(0.0);
        section.style().flex_shrink = Some(0.0);
        section.style().flex_basis = Some(exact_height.into());
    } else {
        section.style().flex_grow = Some(fallback_grow.max(1.0));
        section.style().flex_shrink = Some(1.0);
        section.style().flex_basis = Some(relative(0.0).into());
    }
    section
}

fn px_to_grow(value: Pixels) -> f32 {
    let px_value: f32 = value.into();
    px_value.max(1.0)
}
