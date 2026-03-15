use super::*;

impl MainPaneView {
    fn toggle_show_whitespace(&mut self) {
        self.show_whitespace = !self.show_whitespace;
        // Clear styled text caches so they rebuild with new whitespace setting.
        self.clear_diff_text_style_caches();
        self.clear_conflict_diff_style_caches();
        self.conflict_three_way_segments_cache.clear();
    }

    fn prepare_source_mode_for_diff_search(&mut self, cx: &mut gpui::Context<Self>) {
        if self.is_markdown_preview_active() {
            self.rendered_preview_modes
                .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Source);
            let wants_file_diff = !self.is_file_preview_active()
                && !self.is_worktree_target_directory()
                && self.active_repo().is_some_and(|repo| {
                    Self::is_file_diff_target(repo.diff_state.diff_target.as_ref())
                });
            if wants_file_diff {
                self.ensure_file_diff_cache(cx);
            }
        }
        if self.is_conflict_rendered_preview_active() {
            self.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Text;
        }
    }

    fn activate_diff_search(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.prepare_source_mode_for_diff_search(cx);
        self.diff_search_active = true;
        self.clear_diff_text_query_overlay_cache();
        self.worktree_preview_segments_cache_path = None;
        self.worktree_preview_segments_cache.clear();
        self.clear_conflict_diff_query_overlay_caches();
        self.diff_search_recompute_matches();
        let focus = self.diff_search_input.read(cx).focus_handle();
        window.focus(&focus);
    }

    pub(in crate::view) fn diff_view(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let repo_id = self.active_repo_id();

        // Intentionally no outer panel header; keep diff controls in the inner header.

        let title = self.diff_panel_title(theme);

        let untracked_preview_path = self.untracked_worktree_preview_path();
        let added_preview_path = self.added_file_preview_abs_path();
        let deleted_preview_path = self.deleted_file_preview_abs_path();
        let untracked_directory_notice = self.untracked_directory_notice();

        let is_file_preview = self.is_file_preview_active() && untracked_directory_notice.is_none();

        if is_file_preview {
            if let Some(path) = untracked_preview_path.clone() {
                self.ensure_worktree_preview_loaded(path, cx);
            } else if let Some(path) = added_preview_path.clone().or(deleted_preview_path.clone()) {
                self.ensure_preview_loading(path);
            }
        } else if untracked_directory_notice.is_some()
            && matches!(self.worktree_preview, Loadable::Loading)
        {
            self.worktree_preview_path = None;
            self.worktree_preview = Loadable::NotLoaded;
            self.worktree_preview_segments_cache_path = None;
            self.worktree_preview_segments_cache.clear();
            self.diff_horizontal_min_width = px(0.0);
        }
        let wants_file_diff = !is_file_preview
            && !self.is_worktree_target_directory()
            && self
                .active_repo()
                .is_some_and(|r| Self::is_file_diff_target(r.diff_state.diff_target.as_ref()));

        let repo = self.active_repo();
        let conflict_target = repo.and_then(|repo| {
            let DiffTarget::WorkingTree { path, area } = repo.diff_state.diff_target.as_ref()?
            else {
                return None;
            };
            if *area != DiffArea::Unstaged {
                return None;
            }
            match &repo.status {
                Loadable::Ready(status) => {
                    let conflict = status
                        .unstaged
                        .iter()
                        .find(|e| e.path == *path && e.kind == FileStatusKind::Conflicted)?;
                    Some((path.clone(), conflict.conflict))
                }
                _ => None,
            }
        });
        let (conflict_target_path, conflict_kind) = conflict_target
            .map(|(path, kind)| (Some(path), kind))
            .unwrap_or((None, None));
        // Detect binary from loaded conflict file bytes (has bytes but no text).
        let is_binary_conflict = repo
            .and_then(|r| match &r.conflict_state.conflict_file {
                Loadable::Ready(Some(file)) => {
                    let has_non_text = |bytes: &Option<Vec<u8>>, text: &Option<String>| {
                        bytes.is_some() && text.is_none()
                    };
                    Some(
                        has_non_text(&file.base_bytes, &file.base)
                            || has_non_text(&file.ours_bytes, &file.ours)
                            || has_non_text(&file.theirs_bytes, &file.theirs),
                    )
                }
                _ => None,
            })
            .unwrap_or(false);
        let conflict_strategy = Self::conflict_resolver_strategy(conflict_kind, is_binary_conflict);
        let is_conflict_resolver = conflict_strategy.is_some();
        let is_conflict_compare = conflict_target_path.is_some() && conflict_strategy.is_none();
        let conflict_rendered_preview_active = self.is_conflict_rendered_preview_active();

        let rendered_preview_kind = super::super::diff_target_rendered_preview_kind(
            repo.and_then(|repo| repo.diff_state.diff_target.as_ref()),
        );
        let rendered_view_toggle_kind = super::super::main_diff_rendered_preview_toggle_kind(
            wants_file_diff,
            is_file_preview,
            rendered_preview_kind,
        );
        let is_markdown_preview_view = rendered_view_toggle_kind
            == Some(RenderedPreviewKind::Markdown)
            && self
                .rendered_preview_modes
                .get(RenderedPreviewKind::Markdown)
                == RenderedPreviewMode::Rendered;
        let is_image_diff_loaded = repo
            .is_some_and(|repo| !matches!(repo.diff_state.diff_file_image, Loadable::NotLoaded));
        let is_image_diff_view = wants_file_diff
            && is_image_diff_loaded
            && (!matches!(rendered_preview_kind, Some(RenderedPreviewKind::Svg))
                || self.rendered_preview_modes.get(RenderedPreviewKind::Svg)
                    == RenderedPreviewMode::Rendered);

        let (prev_file_btn, next_file_btn) = self.diff_prev_next_file_buttons(repo_id, theme, cx);

        let mut controls = div().flex().items_center().gap_1();
        let is_simple_conflict_strategy = matches!(
            self.conflict_resolver.strategy,
            Some(
                gitcomet_core::conflict_session::ConflictResolverStrategy::BinarySidePick
                    | gitcomet_core::conflict_session::ConflictResolverStrategy::TwoWayKeepDelete
                    | gitcomet_core::conflict_session::ConflictResolverStrategy::DecisionOnly
            )
        );
        if is_conflict_resolver && is_simple_conflict_strategy {
            // Binary, keep/delete, and decision-only conflicts handle actions
            // inline in their dedicated panels; only show file navigation.
            controls = controls
                .when_some(prev_file_btn, |d, btn| d.child(btn))
                .when_some(next_file_btn, |d, btn| d.child(btn));
            let conflict_count = self.conflict_resolver_conflict_count();
            if conflict_count > 0 {
                let resolved_count = self.conflict_resolver_resolved_count();
                let unresolved_count = conflict_count.saturating_sub(resolved_count);
                controls = controls.child(
                    div()
                        .text_xs()
                        .text_color(if unresolved_count == 0 {
                            theme.colors.success
                        } else {
                            theme.colors.text_muted
                        })
                        .child(format!("Resolved {resolved_count}/{conflict_count}")),
                );
                if unresolved_count > 0 {
                    controls = controls.child(
                        div()
                            .text_xs()
                            .text_color(theme.colors.danger)
                            .child(format!("{unresolved_count} unresolved")),
                    );
                }
            }
        } else if is_conflict_resolver {
            controls = controls
                .when_some(prev_file_btn, |d, btn| d.child(btn))
                .when(!conflict_rendered_preview_active, |d| {
                    let nav_entries = self.conflict_nav_entries();
                    let current_nav_ix = self.conflict_resolver.nav_anchor.unwrap_or(0);
                    let can_nav_prev =
                        diff_navigation::diff_nav_prev_target(&nav_entries, current_nav_ix)
                            .is_some();
                    let can_nav_next =
                        diff_navigation::diff_nav_next_target(&nav_entries, current_nav_ix)
                            .is_some();

                    d.child(
                        components::Button::new("conflict_prev", "Prev")
                            .end_slot(Self::diff_nav_hotkey_hint(theme, "F2"))
                            .style(components::ButtonStyle::Outlined)
                            .disabled(!can_nav_prev)
                            .on_click(theme, cx, |this, _e, _w, cx| {
                                this.conflict_jump_prev(cx);
                                cx.notify();
                            }),
                    )
                    .child(
                        components::Button::new("conflict_next", "Next")
                            .end_slot(Self::diff_nav_hotkey_hint(theme, "F3"))
                            .style(components::ButtonStyle::Outlined)
                            .disabled(!can_nav_next)
                            .on_click(theme, cx, |this, _e, _w, cx| {
                                this.conflict_jump_next(cx);
                                cx.notify();
                            }),
                    )
                })
                .when_some(next_file_btn, |d, btn| d.child(btn));

            let resolved_output_text = self
                .conflict_resolver_input
                .read_with(cx, |i, _| i.text().to_string());
            let stage_safety = conflict_resolver::conflict_stage_safety_check(
                &resolved_output_text,
                &self.conflict_resolver.marker_segments,
            );

            if stage_safety.has_conflict_markers {
                controls = controls.child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.danger)
                        .child("markers remain"),
                );
            }

            if let (Some(repo_id), Some(path)) = (repo_id, conflict_target_path.clone()) {
                let focused_mergetool_mode = self.view_mode == GitCometViewMode::FocusedMergetool;
                let save_label = if focused_mergetool_mode {
                    "Save & close"
                } else {
                    "Save"
                };
                let save_path = path.clone();
                controls = controls
                    .child(
                        components::Button::new("conflict_save", save_label)
                            .style(components::ButtonStyle::Outlined)
                            .on_click(theme, cx, move |this, _e, _w, cx| {
                                if this.view_mode == GitCometViewMode::FocusedMergetool {
                                    this.focused_mergetool_save_and_exit(
                                        repo_id,
                                        save_path.clone(),
                                        cx,
                                    );
                                    return;
                                }
                                let text = this
                                    .conflict_resolver_input
                                    .read_with(cx, |i, _| i.text().to_string());
                                this.conflict_resolver_sync_session_resolutions_from_output(&text);
                                this.store.dispatch(Msg::SaveWorktreeFile {
                                    repo_id,
                                    path: save_path.clone(),
                                    contents: text,
                                    stage: false,
                                });
                            }),
                    )
                    .when(show_conflict_save_stage_action(self.view_mode), |d| {
                        let save_path = path.clone();
                        d.child(
                            components::Button::new("conflict_save_stage", "Save & stage")
                                .style(components::ButtonStyle::Filled)
                                .on_click(theme, cx, move |this, e, window, cx| {
                                    let text = this
                                        .conflict_resolver_input
                                        .read_with(cx, |i, _| i.text().to_string());
                                    let stage_safety =
                                        conflict_resolver::conflict_stage_safety_check(
                                            &text,
                                            &this.conflict_resolver.marker_segments,
                                        );
                                    if stage_safety.requires_confirmation() {
                                        this.open_popover_at(
                                            PopoverKind::ConflictSaveStageConfirm {
                                                repo_id,
                                                path: save_path.clone(),
                                                has_conflict_markers: stage_safety
                                                    .has_conflict_markers,
                                                unresolved_blocks: stage_safety.unresolved_blocks,
                                            },
                                            e.position(),
                                            window,
                                            cx,
                                        );
                                    } else {
                                        this.conflict_resolver_sync_session_resolutions_from_output(
                                            &text,
                                        );
                                        this.store.dispatch(Msg::SaveWorktreeFile {
                                            repo_id,
                                            path: save_path.clone(),
                                            contents: text,
                                            stage: true,
                                        });
                                    }
                                }),
                        )
                    });
            }
        } else if !is_file_preview {
            controls = controls.when_some(prev_file_btn, |d, btn| d.child(btn));

            if !is_image_diff_view && !is_markdown_preview_view {
                let nav_entries = self.diff_nav_entries();
                let current_nav_ix = self.diff_selection_anchor.unwrap_or(0);
                let can_nav_prev =
                    diff_navigation::diff_nav_prev_target(&nav_entries, current_nav_ix).is_some();
                let can_nav_next =
                    diff_navigation::diff_nav_next_target(&nav_entries, current_nav_ix).is_some();

                let prev_hunk_btn = components::Button::new("diff_prev_hunk", "Prev")
                    .end_slot(Self::diff_nav_hotkey_hint(theme, "F2"))
                    .style(components::ButtonStyle::Outlined)
                    .disabled(!can_nav_prev)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.diff_jump_prev();
                        cx.notify();
                    })
                    .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                        let text: SharedString = "Previous change (F2 / Shift+F7 / Alt+Up)".into();
                        let mut changed = false;
                        if *hovering {
                            changed |= this.set_tooltip_text_if_changed(Some(text.clone()), cx);
                        } else {
                            changed |= this.clear_tooltip_if_matches(&text, cx);
                        }
                        if changed {
                            cx.notify();
                        }
                    }));

                let next_hunk_btn = components::Button::new("diff_next_hunk", "Next")
                    .end_slot(Self::diff_nav_hotkey_hint(theme, "F3"))
                    .style(components::ButtonStyle::Outlined)
                    .disabled(!can_nav_next)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.diff_jump_next();
                        cx.notify();
                    })
                    .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                        let text: SharedString = "Next change (F3 / F7 / Alt+Down)".into();
                        let mut changed = false;
                        if *hovering {
                            changed |= this.set_tooltip_text_if_changed(Some(text.clone()), cx);
                        } else {
                            changed |= this.clear_tooltip_if_matches(&text, cx);
                        }
                        if changed {
                            cx.notify();
                        }
                    }));

                let view_toggle_selected_bg =
                    with_alpha(theme.colors.accent, if theme.is_dark { 0.26 } else { 0.20 });
                let view_toggle_border = with_alpha(
                    theme.colors.text_muted,
                    if theme.is_dark { 0.38 } else { 0.28 },
                );
                let view_toggle_divider = with_alpha(view_toggle_border, 0.90);
                let diff_inline_btn = components::Button::new("diff_inline", "Inline")
                    .borderless()
                    .style(components::ButtonStyle::Subtle)
                    .selected(self.diff_view == DiffViewMode::Inline)
                    .selected_bg(view_toggle_selected_bg)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.diff_view = DiffViewMode::Inline;
                        this.clear_diff_text_style_caches();
                        if this.diff_search_active
                            && !this.diff_search_query.as_ref().trim().is_empty()
                        {
                            this.diff_search_recompute_matches();
                        }
                        cx.notify();
                    })
                    .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                        let text: SharedString = "Inline diff view (Alt+I)".into();
                        let mut changed = false;
                        if *hovering {
                            changed |= this.set_tooltip_text_if_changed(Some(text.clone()), cx);
                        } else {
                            changed |= this.clear_tooltip_if_matches(&text, cx);
                        }
                        if changed {
                            cx.notify();
                        }
                    }));

                let diff_split_btn = components::Button::new("diff_split", "Split")
                    .borderless()
                    .style(components::ButtonStyle::Subtle)
                    .selected(self.diff_view == DiffViewMode::Split)
                    .selected_bg(view_toggle_selected_bg)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.diff_view = DiffViewMode::Split;
                        this.clear_diff_text_style_caches();
                        if this.diff_search_active
                            && !this.diff_search_query.as_ref().trim().is_empty()
                        {
                            this.diff_search_recompute_matches();
                        }
                        cx.notify();
                    })
                    .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                        let text: SharedString = "Split diff view (Alt+S)".into();
                        let mut changed = false;
                        if *hovering {
                            changed |= this.set_tooltip_text_if_changed(Some(text.clone()), cx);
                        } else {
                            changed |= this.clear_tooltip_if_matches(&text, cx);
                        }
                        if changed {
                            cx.notify();
                        }
                    }));

                let view_toggle = div()
                    .id("diff_view_toggle")
                    .flex()
                    .items_center()
                    .h(px(components::CONTROL_HEIGHT_PX))
                    .rounded(px(theme.radii.row))
                    .border_1()
                    .border_color(view_toggle_border)
                    .bg(gpui::rgba(0x00000000))
                    .overflow_hidden()
                    .p(px(1.0))
                    .child(diff_inline_btn)
                    .child(div().h_full().w(px(1.0)).bg(view_toggle_divider))
                    .child(diff_split_btn);

                controls = controls
                    .child(prev_hunk_btn)
                    .child(next_hunk_btn)
                    .when_some(next_file_btn, |d, btn| d.child(btn))
                    .child(view_toggle)
                    .when(!wants_file_diff, |controls| {
                        controls.child(
                            components::Button::new("diff_hunks", "Hunks")
                                .style(components::ButtonStyle::Outlined)
                                .on_click(theme, cx, |this, e, window, cx| {
                                    this.open_popover_at(
                                        PopoverKind::DiffHunks,
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                    cx.notify();
                                })
                                .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                                    let text: SharedString = "Jump to hunk (Alt+H)".into();
                                    let mut changed = false;
                                    if *hovering {
                                        changed |= this
                                            .set_tooltip_text_if_changed(Some(text.clone()), cx);
                                    } else {
                                        changed |= this.clear_tooltip_if_matches(&text, cx);
                                    }
                                    if changed {
                                        cx.notify();
                                    }
                                })),
                        )
                    });
            } else {
                controls = controls.when_some(next_file_btn, |d, btn| d.child(btn));
            }
        } else {
            controls = controls
                .when_some(prev_file_btn, |d, btn| d.child(btn))
                .when_some(next_file_btn, |d, btn| d.child(btn));
        }

        if !is_conflict_resolver && let Some(preview_kind) = rendered_view_toggle_kind {
            let preview_mode = self.rendered_preview_modes.get(preview_kind);
            controls = controls.child(
                div()
                    .id(preview_kind.toggle_id())
                    .debug_selector(move || preview_kind.toggle_id().to_string())
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        components::Button::new(
                            preview_kind.rendered_button_id(),
                            preview_kind.rendered_label(),
                        )
                        .style(if preview_mode == RenderedPreviewMode::Rendered {
                            components::ButtonStyle::Filled
                        } else {
                            components::ButtonStyle::Outlined
                        })
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.rendered_preview_modes
                                .set(preview_kind, RenderedPreviewMode::Rendered);
                            cx.notify();
                        }),
                    )
                    .child(
                        components::Button::new(
                            preview_kind.source_button_id(),
                            preview_kind.source_label(),
                        )
                        .style(if preview_mode == RenderedPreviewMode::Source {
                            components::ButtonStyle::Filled
                        } else {
                            components::ButtonStyle::Outlined
                        })
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.rendered_preview_modes
                                .set(preview_kind, RenderedPreviewMode::Source);
                            cx.notify();
                        }),
                    ),
            );
        }

        if let Some(repo_id) = repo_id {
            controls = controls.child(
                components::Button::new("diff_close", "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.text_muted,
                        px(12.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        this.clear_diff_selection_or_exit(repo_id, cx);
                        cx.notify();
                    })
                    .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                        let text: SharedString = "Close diff".into();
                        let mut changed = false;
                        if *hovering {
                            changed |= this.set_tooltip_text_if_changed(Some(text.clone()), cx);
                        } else {
                            changed |= this.clear_tooltip_if_matches(&text, cx);
                        }
                        if changed {
                            cx.notify();
                        }
                    })),
            );
        }

        if self.diff_search_active {
            let query = self.diff_search_query.as_ref().trim();
            let match_label: SharedString = if query.is_empty() {
                "Type to search".into()
            } else if self.diff_search_matches.is_empty() {
                "No matches".into()
            } else {
                let ix = self
                    .diff_search_match_ix
                    .unwrap_or(0)
                    .min(self.diff_search_matches.len().saturating_sub(1));
                format!("{}/{}", ix + 1, self.diff_search_matches.len()).into()
            };

            controls = controls
                .child(
                    div()
                        .w(px(240.0))
                        .min_w(px(120.0))
                        .child(self.diff_search_input.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child(match_label),
                )
                .child(
                    components::Button::new("diff_search_close", "")
                        .start_slot(svg_icon(
                            "icons/generic_close.svg",
                            theme.colors.text_muted,
                            px(12.0),
                        ))
                        .style(components::ButtonStyle::Transparent)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.diff_search_active = false;
                            this.diff_search_matches.clear();
                            this.diff_search_match_ix = None;
                            this.clear_diff_text_query_overlay_cache();
                            this.clear_worktree_preview_segments_cache();
                            this.clear_conflict_diff_query_overlay_caches();
                            window.focus(&this.diff_panel_focus_handle);
                            cx.notify();
                        }),
                );
        }

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(components::CONTROL_HEIGHT_MD_PX))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(div().flex_1().min_w(px(0.0)).overflow_hidden().child(title)),
            )
            .child(controls);

        let body: AnyElement = if let Some(message) = untracked_directory_notice {
            components::empty_state(theme, "Directory", message).into_any_element()
        } else if is_file_preview {
            if added_preview_path.is_some() || deleted_preview_path.is_some() {
                self.try_populate_worktree_preview_from_diff_file(cx);
            }
            if is_markdown_preview_view {
                match &self.worktree_preview {
                    Loadable::NotLoaded | Loadable::Loading => {
                        components::empty_state(theme, "Preview", "Loading").into_any_element()
                    }
                    Loadable::Error(e) => {
                        components::empty_state(theme, "Preview", e.clone()).into_any_element()
                    }
                    Loadable::Ready(_) => {
                        self.ensure_single_markdown_preview_cache(cx);
                        match &self.worktree_markdown_preview {
                            Loadable::NotLoaded | Loadable::Loading => {
                                components::empty_state(theme, "Preview", "Loading")
                                    .into_any_element()
                            }
                            Loadable::Error(e) => {
                                components::empty_state(theme, "Preview", e.clone())
                                    .into_any_element()
                            }
                            Loadable::Ready(document) => {
                                if document.rows.is_empty() {
                                    let message = if self.worktree_preview_line_count() == Some(0) {
                                        "Empty file."
                                    } else {
                                        "Nothing to render."
                                    };
                                    components::empty_state(theme, "Preview", message)
                                        .into_any_element()
                                } else {
                                    let list = uniform_list(
                                        "worktree_markdown_preview_list",
                                        document.rows.len(),
                                        cx.processor(Self::render_markdown_preview_rows),
                                    )
                                    .h_full()
                                    .min_h(px(0.0))
                                    .track_scroll(self.worktree_preview_scroll.clone())
                                    .with_horizontal_sizing_behavior(
                                        gpui::ListHorizontalSizingBehavior::Unconstrained,
                                    );

                                    let scroll_handle =
                                        self.worktree_preview_scroll.0.borrow().base_handle.clone();
                                    div()
                                        .id("worktree_markdown_preview_scroll_container")
                                        .debug_selector(|| {
                                            "worktree_markdown_preview_scroll_container".to_string()
                                        })
                                        .relative()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .child(list)
                                        .child(
                                            components::Scrollbar::new(
                                                "worktree_markdown_preview_scrollbar",
                                                scroll_handle.clone(),
                                            )
                                            .render(theme),
                                        )
                                        .child(
                                            components::Scrollbar::horizontal(
                                                "worktree_markdown_preview_hscrollbar",
                                                scroll_handle,
                                            )
                                            .always_visible()
                                            .render(theme),
                                        )
                                        .into_any_element()
                                }
                            }
                        }
                    }
                }
            } else {
                match &self.worktree_preview {
                    Loadable::NotLoaded | Loadable::Loading => {
                        components::empty_state(theme, "File", "Loading").into_any_element()
                    }
                    Loadable::Error(e) => {
                        self.diff_raw_input.update(cx, |input, cx| {
                            input.set_theme(theme, cx);
                            input.set_text(e.clone(), cx);
                            input.set_read_only(true, cx);
                        });
                        div()
                            .id("worktree_preview_error_scroll")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .child(self.diff_raw_input.clone())
                            .into_any_element()
                    }
                    Loadable::Ready(line_count) => {
                        if *line_count == 0 {
                            components::empty_state(theme, "File", "Empty file.").into_any_element()
                        } else {
                            let list = uniform_list(
                                "worktree_preview_list",
                                *line_count,
                                cx.processor(Self::render_worktree_preview_rows),
                            )
                            .h_full()
                            .min_h(px(0.0))
                            .track_scroll(self.worktree_preview_scroll.clone())
                            .with_horizontal_sizing_behavior(
                                gpui::ListHorizontalSizingBehavior::Unconstrained,
                            );

                            let scroll_handle =
                                self.worktree_preview_scroll.0.borrow().base_handle.clone();
                            div()
                                .id("worktree_preview_scroll_container")
                                .debug_selector(|| "worktree_preview_scroll_container".to_string())
                                .relative()
                                .h_full()
                                .min_h(px(0.0))
                                .child(list)
                                .child(
                                    components::Scrollbar::new(
                                        "worktree_preview_scrollbar",
                                        scroll_handle.clone(),
                                    )
                                    .render(theme),
                                )
                                .child(
                                    components::Scrollbar::horizontal(
                                        "worktree_preview_hscrollbar",
                                        scroll_handle,
                                    )
                                    .always_visible()
                                    .render(theme),
                                )
                                .into_any_element()
                        }
                    }
                }
            }
        } else if is_conflict_resolver {
            match (repo, conflict_target_path) {
                (None, _) => {
                    components::empty_state(theme, "Resolve", "No repository.").into_any_element()
                }
                (_, None) => {
                    components::empty_state(theme, "Resolve", "No conflicted file selected.")
                        .into_any_element()
                }
                (Some(repo), Some(path)) => {
                    let title: SharedString =
                        format!("Resolve conflict: {}", self.cached_path_display(&path)).into();
                    if let Some(repo_id) = repo_id {
                        match &repo.conflict_state.conflict_file {
                            Loadable::NotLoaded | Loadable::Loading => {
                                components::empty_state(theme, title, "Loading conflict data…")
                                    .into_any_element()
                            }
                            Loadable::Error(e) => {
                                components::empty_state(theme, title, e.clone()).into_any_element()
                            }
                            Loadable::Ready(None) => {
                                components::empty_state(theme, title, "No conflict data.")
                                    .into_any_element()
                            }
                            Loadable::Ready(Some(file))
                                if self.conflict_resolver.is_binary_conflict =>
                            {
                                // Binary/non-UTF8 side-pick resolver panel.
                                let file_clone = file.clone();
                                self.render_binary_conflict_resolver(
                                    theme,
                                    repo_id,
                                    path,
                                    &file_clone,
                                    cx,
                                )
                            }
                            Loadable::Ready(Some(file))
                                if matches!(
                                    self.conflict_resolver.strategy,
                                    Some(gitcomet_core::conflict_session::ConflictResolverStrategy::TwoWayKeepDelete)
                                ) =>
                            {
                                // Keep/delete resolver for modify/delete conflicts.
                                let file_clone = file.clone();
                                let kind = self.conflict_resolver.conflict_kind.unwrap_or(
                                    gitcomet_core::domain::FileConflictKind::DeletedByUs,
                                );
                                self.render_keep_delete_conflict_resolver(
                                    theme, repo_id, path, &file_clone, kind, cx,
                                )
                            }
                            Loadable::Ready(Some(file))
                                if matches!(
                                    self.conflict_resolver.strategy,
                                    Some(gitcomet_core::conflict_session::ConflictResolverStrategy::DecisionOnly)
                                ) =>
                            {
                                // Decision-only resolver for BothDeleted conflicts.
                                let file_clone = file.clone();
                                self.render_decision_conflict_resolver(
                                    theme, repo_id, path, &file_clone, cx,
                                )
                            }
                            Loadable::Ready(Some(file)) => {
                            let base = file.base.clone().unwrap_or_default();
                            let local = file.ours.clone().unwrap_or_default();
                            let remote = file.theirs.clone().unwrap_or_default();
                            let has_current = file.current.is_some();

                            let view_mode = self.conflict_resolver.view_mode;
                            let mode = self.conflict_resolver.diff_mode;

                            let toggle_mode_split =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_set_mode(ConflictDiffMode::Split, cx);
                                };
                            let toggle_mode_inline =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_set_mode(ConflictDiffMode::Inline, cx);
                                };

                            let set_view_three_way =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_set_view_mode(
                                        ConflictResolverViewMode::ThreeWay,
                                        cx,
                                    );
                                };
                            let set_view_two_way =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_set_view_mode(
                                        ConflictResolverViewMode::TwoWayDiff,
                                        cx,
                                    );
                                };

                            let reset_from_markers =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_reset_output_from_markers(cx);
                                };

                            let view_toggle_selected_bg = with_alpha(
                                theme.colors.accent,
                                if theme.is_dark { 0.26 } else { 0.20 },
                            );
                            let view_toggle_border = with_alpha(
                                theme.colors.text_muted,
                                if theme.is_dark { 0.38 } else { 0.28 },
                            );
                            let view_toggle_divider = with_alpha(view_toggle_border, 0.90);
                            let show_whitespace = self.show_whitespace;
                            let ws_pill_border_hover = if show_whitespace {
                                theme.colors.accent
                            } else {
                                view_toggle_border
                            };
                            let ws_pill_text = if theme.is_dark {
                                theme.colors.text
                            } else {
                                gpui::rgba(0xffffffff)
                            };
                            let show_whitespace_control = div()
                                .id("conflict_show_whitespace_pill")
                                .h(px(components::CONTROL_HEIGHT_PX))
                                .px(px(8.0))
                                .py(px(2.0))
                                .rounded(px(theme.radii.pill))
                                .bg(gpui::rgba(0x000000ff))
                                .border_1()
                                .border_color(gpui::rgba(0x00000000))
                                .text_xs()
                                .text_color(ws_pill_text)
                                .cursor(CursorStyle::PointingHand)
                                .hover(move |pill| pill.border_color(ws_pill_border_hover))
                                .active(move |pill| pill.border_color(ws_pill_border_hover))
                                .on_any_mouse_down(|_e, _w, cx| cx.stop_propagation())
                                .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
                                    this.toggle_show_whitespace();
                                    cx.notify();
                                }))
                                .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                                    let text: SharedString = "Show whitespace (Alt+W)".into();
                                    let mut changed = false;
                                    if *hovering {
                                        changed |=
                                            this.set_tooltip_text_if_changed(Some(text.clone()), cx);
                                    } else {
                                        changed |= this.clear_tooltip_if_matches(&text, cx);
                                    }
                                    if changed {
                                        cx.notify();
                                    }
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child("Show whitespace")
                                        .when(show_whitespace, |d| {
                                            d.child(
                                                div().child(svg_icon(
                                                    "icons/check.svg",
                                                    theme.colors.success,
                                                    px(12.0),
                                                )),
                                            )
                                        }),
                                );

                            let view_mode_controls = div()
                                .id("conflict_view_mode_toggle")
                                .flex()
                                .items_center()
                                .h(px(components::CONTROL_HEIGHT_PX))
                                .rounded(px(theme.radii.row))
                                .border_1()
                                .border_color(view_toggle_border)
                                .bg(gpui::rgba(0x00000000))
                                .overflow_hidden()
                                .p(px(1.0))
                                .child(
                                    components::Button::new("conflict_view_three_way", "3-way")
                                        .borderless()
                                        .style(components::ButtonStyle::Subtle)
                                        .selected(view_mode == ConflictResolverViewMode::ThreeWay)
                                        .selected_bg(view_toggle_selected_bg)
                                        .on_click(theme, cx, set_view_three_way),
                                )
                                .child(div().h_full().w(px(1.0)).bg(view_toggle_divider))
                                .child(
                                    components::Button::new("conflict_view_two_way", "2-way")
                                        .borderless()
                                        .style(components::ButtonStyle::Subtle)
                                        .selected(view_mode == ConflictResolverViewMode::TwoWayDiff)
                                        .selected_bg(view_toggle_selected_bg)
                                        .on_click(theme, cx, set_view_two_way),
                                );

                            let diff_len = match view_mode {
                                ConflictResolverViewMode::ThreeWay => {
                                    self.conflict_resolver.three_way_visible_map.len()
                                }
                                ConflictResolverViewMode::TwoWayDiff => match mode {
                                    ConflictDiffMode::Split => {
                                        self.conflict_resolver.diff_visible_row_indices.len()
                                    }
                                    ConflictDiffMode::Inline => {
                                        self.conflict_resolver.inline_visible_row_indices.len()
                                    }
                                },
                            };

                            let mode_controls = div()
                                .id("conflict_mode_toggle")
                                .flex()
                                .items_center()
                                .h(px(components::CONTROL_HEIGHT_PX))
                                .rounded(px(theme.radii.row))
                                .border_1()
                                .border_color(view_toggle_border)
                                .bg(gpui::rgba(0x00000000))
                                .overflow_hidden()
                                .p(px(1.0))
                                .child(
                                    components::Button::new("conflict_mode_split", "Split")
                                        .borderless()
                                        .style(components::ButtonStyle::Subtle)
                                        .selected(mode == ConflictDiffMode::Split)
                                        .selected_bg(view_toggle_selected_bg)
                                        .on_click(theme, cx, toggle_mode_split),
                                )
                                .child(div().h_full().w(px(1.0)).bg(view_toggle_divider))
                                .child(
                                    components::Button::new("conflict_mode_inline", "Inline")
                                        .borderless()
                                        .style(components::ButtonStyle::Subtle)
                                        .selected(mode == ConflictDiffMode::Inline)
                                        .selected_bg(view_toggle_selected_bg)
                                        .on_click(theme, cx, toggle_mode_inline),
                                );

                            let conflict_count = self.conflict_resolver_conflict_count();
                            let active_conflict = self.conflict_resolver.active_conflict;
                            let has_conflicts = conflict_count > 0;
                            let resolved_count = self.conflict_resolver_resolved_count();
                            let unresolved_count = conflict_count - resolved_count;
                            let active_autosolve_trace = repo
                                .conflict_state.conflict_session
                                .as_ref()
                                .and_then(|session| {
                                    conflict_resolver::active_conflict_autosolve_trace_label(
                                        session,
                                        &self.conflict_resolver.conflict_region_indices,
                                        active_conflict,
                                    )
                                })
                                .map(SharedString::from);

                            let auto_resolve =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_auto_resolve(cx);
                                };
                            let toggle_hide_resolved =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_toggle_hide_resolved(cx);
                                };
                            let hide_resolved = self.conflict_resolver.hide_resolved;

                            let start_controls = div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .when(has_conflicts, |d| {
                                    let nav_label: SharedString = format!(
                                        "Conflict {}/{}",
                                        active_conflict + 1,
                                        conflict_count
                                    )
                                    .into();
                                    let resolved_label: SharedString =
                                        format!("Resolved {}/{}", resolved_count, conflict_count)
                                            .into();

                                    let mut d = d.child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.colors.text_muted)
                                            .child(nav_label),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(if unresolved_count == 0 {
                                                theme.colors.success
                                            } else {
                                                theme.colors.text_muted
                                            })
                                            .child(resolved_label),
                                    );
                                    if let Some(label) = active_autosolve_trace.as_ref() {
                                        d = d.child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.colors.accent)
                                                .child(label.clone()),
                                        );
                                    }
                                    d
                                })
                                .child(
                                    components::Button::new(
                                        "conflict_reset_markers",
                                        "Reset from markers",
                                    )
                                    .style(components::ButtonStyle::Transparent)
                                    .disabled(!has_current)
                                    .on_click(
                                        theme,
                                        cx,
                                        reset_from_markers,
                                    ),
                                )
                                .when(has_conflicts && unresolved_count > 0, |d| {
                                    d.child(div().w(px(1.0)).h(px(12.0)).bg(theme.colors.border))
                                        .child(
                                            components::Button::new(
                                                "conflict_auto_resolve",
                                                "Auto-resolve",
                                            )
                                            .style(components::ButtonStyle::Outlined)
                                            .on_click(
                                                theme,
                                                cx,
                                                auto_resolve,
                                            ),
                                        )
                                })
                                .when(has_conflicts && resolved_count > 0, |d| {
                                    d.child(
                                        components::Button::new(
                                            "conflict_hide_resolved",
                                            if hide_resolved {
                                                "Show resolved"
                                            } else {
                                                "Hide resolved"
                                            },
                                        )
                                        .style(if hide_resolved {
                                            components::ButtonStyle::Outlined
                                        } else {
                                            components::ButtonStyle::Transparent
                                        })
                                        .on_click(
                                            theme,
                                            cx,
                                            toggle_hide_resolved,
                                        ),
                                    )
                                });

                            let preview_kind = super::super::preview_path_rendered_kind(&path);
                            let show_preview_toggle = preview_kind.is_some();
                            let preview_mode = self.conflict_resolver.resolver_preview_mode;
                            let is_rendered_preview_active =
                                show_preview_toggle
                                    && preview_mode == ConflictResolverPreviewMode::Preview;

                            let preview_toggle = show_preview_toggle.then(|| {
                                let view_toggle_border = theme.colors.border;
                                let view_toggle_selected_bg = theme.colors.active;
                                let view_toggle_divider = theme.colors.border;
                                div()
                                    .id("conflict_preview_toggle")
                                    .flex()
                                    .items_center()
                                    .h(px(components::CONTROL_HEIGHT_PX))
                                    .rounded(px(theme.radii.row))
                                    .border_1()
                                    .border_color(view_toggle_border)
                                    .bg(gpui::rgba(0x00000000))
                                    .overflow_hidden()
                                    .p(px(1.0))
                                    .child(
                                        components::Button::new("conflict_preview_text", "Text")
                                            .borderless()
                                            .style(components::ButtonStyle::Subtle)
                                            .selected(preview_mode == ConflictResolverPreviewMode::Text)
                                            .selected_bg(view_toggle_selected_bg)
                                            .on_click(theme, cx, |this, _e, _w, cx| {
                                                this.conflict_resolver.resolver_preview_mode =
                                                    ConflictResolverPreviewMode::Text;
                                                cx.notify();
                                            }),
                                    )
                                    .child(div().h_full().w(px(1.0)).bg(view_toggle_divider))
                                    .child(
                                        components::Button::new(
                                            "conflict_preview_preview",
                                            preview_kind
                                                .map(RenderedPreviewKind::rendered_label)
                                                .unwrap_or("Preview"),
                                        )
                                        .borderless()
                                        .style(components::ButtonStyle::Subtle)
                                        .selected(
                                            preview_mode == ConflictResolverPreviewMode::Preview,
                                        )
                                        .selected_bg(view_toggle_selected_bg)
                                        .on_click(theme, cx, |this, _e, _w, cx| {
                                            this.conflict_resolver.resolver_preview_mode =
                                                ConflictResolverPreviewMode::Preview;
                                            cx.notify();
                                        }),
                                    )
                            });

                            let top_header = div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div().text_xs().text_color(theme.colors.text_muted).child(
                                                if is_rendered_preview_active {
                                                    "Preview inputs (base / local / remote)"
                                                } else {
                                                    match view_mode {
                                                        ConflictResolverViewMode::ThreeWay => {
                                                            "Merge inputs (base / local / remote)"
                                                        }
                                                        ConflictResolverViewMode::TwoWayDiff => {
                                                            "Diff (local ↔ remote)"
                                                        }
                                                    }
                                                },
                                            ),
                                        )
                                        .when_some(preview_toggle, |d, toggle| d.child(toggle)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .when(!is_rendered_preview_active, |d| {
                                            d.child(show_whitespace_control)
                                                .when(
                                                    view_mode
                                                        == ConflictResolverViewMode::TwoWayDiff,
                                                    |d| d.child(mode_controls),
                                                )
                                                .child(view_mode_controls)
                                        }),
                                );

                            // Compute three-way column widths
                            let handle_w = px(PANE_RESIZE_HANDLE_PX);
                            let min_col_w = px(DIFF_SPLIT_COL_MIN_PX);
                            let main_w = self.main_pane_content_width(cx);
                            let available = (main_w - handle_w * 2.0).max(px(0.0));
                            let ratios = self.conflict_three_way_col_ratios;
                            let col_a_w = if available <= min_col_w * 3.0 {
                                available / 3.0
                            } else {
                                (available * ratios[0])
                                    .max(min_col_w)
                                    .min(available - min_col_w * 2.0)
                            };
                            let col_b_w = if available <= min_col_w * 3.0 {
                                available / 3.0
                            } else {
                                (available * (ratios[1] - ratios[0]))
                                    .max(min_col_w)
                                    .min(available - col_a_w - min_col_w)
                            };
                            let col_c_w = (available - col_a_w - col_b_w).max(px(0.0));
                            self.conflict_three_way_col_widths = [col_a_w, col_b_w, col_c_w];

                            // Compute two-way diff split column widths
                            {
                                let two_available = (main_w - handle_w).max(px(0.0));
                                let two_ratio = self.conflict_diff_split_ratio;
                                let left_w = if two_available <= min_col_w * 2.0 {
                                    two_available * 0.5
                                } else {
                                    (two_available * two_ratio)
                                        .max(min_col_w)
                                        .min(two_available - min_col_w)
                                };
                                let right_w = two_available - left_w;
                                self.conflict_diff_split_col_widths = [left_w, right_w];
                            }

                            let conflict_hsplit_resize_handle =
                                |id: &'static str, which: ConflictHSplitResizeHandle| {
                                    div()
                                        .id(id)
                                        .w(handle_w)
                                        .h_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor(CursorStyle::ResizeLeftRight)
                                        .hover(move |s| s.bg(with_alpha(theme.colors.hover, 0.65)))
                                        .active(move |s| s.bg(theme.colors.active))
                                        .child(div().w(px(1.0)).h_full().bg(theme.colors.border))
                                        .on_drag(which, |_handle, _offset, _window, cx| {
                                            cx.new(|_cx| ConflictHSplitResizeDragGhost)
                                        })
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                                                cx.stop_propagation();
                                                this.conflict_hsplit_resize =
                                                    Some(ConflictHSplitResizeState {
                                                        handle: which,
                                                        start_x: e.position.x,
                                                        start_ratios: this
                                                            .conflict_three_way_col_ratios,
                                                    });
                                                cx.notify();
                                            }),
                                        )
                                        .on_drag_move(cx.listener(
                                            move |this,
                                                  e: &gpui::DragMoveEvent<
                                                ConflictHSplitResizeHandle,
                                            >,
                                                  _w,
                                                  cx| {
                                                let Some(state) = this.conflict_hsplit_resize
                                                else {
                                                    return;
                                                };
                                                if state.handle != *e.drag(cx) {
                                                    return;
                                                }

                                                let main_w = this.main_pane_content_width(cx);
                                                let avail = (main_w - handle_w * 2.0).max(px(0.0));
                                                if avail <= min_col_w * 3.0 {
                                                    this.conflict_three_way_col_ratios =
                                                        [1.0 / 3.0, 2.0 / 3.0];
                                                    cx.notify();
                                                    return;
                                                }

                                                let dx = e.event.position.x - state.start_x;
                                                let mut r = state.start_ratios;
                                                match state.handle {
                                                    ConflictHSplitResizeHandle::First => {
                                                        let new_pos = (avail * r[0] + dx)
                                                            .max(min_col_w)
                                                            .min(avail - min_col_w * 2.0);
                                                        r[0] = (new_pos / avail).clamp(0.0, 1.0);
                                                        // Ensure second divider stays valid
                                                        let min_r1 = r[0] + (min_col_w / avail);
                                                        if r[1] < min_r1 {
                                                            r[1] =
                                                                min_r1.min(1.0 - min_col_w / avail);
                                                        }
                                                    }
                                                    ConflictHSplitResizeHandle::Second => {
                                                        let new_pos = (avail * r[1] + dx)
                                                            .max(min_col_w * 2.0)
                                                            .min(avail - min_col_w);
                                                        r[1] = (new_pos / avail).clamp(0.0, 1.0);
                                                        // Ensure first divider stays valid
                                                        let max_r0 = r[1] - (min_col_w / avail);
                                                        if r[0] > max_r0 {
                                                            r[0] = max_r0.max(min_col_w / avail);
                                                        }
                                                    }
                                                }
                                                this.conflict_three_way_col_ratios = r;
                                                cx.notify();
                                            },
                                        ))
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|this, _e, _w, cx| {
                                                this.conflict_hsplit_resize = None;
                                                cx.notify();
                                            }),
                                        )
                                        .on_mouse_up_out(
                                            MouseButton::Left,
                                            cx.listener(|this, _e, _w, cx| {
                                                this.conflict_hsplit_resize = None;
                                                cx.notify();
                                            }),
                                        )
                                };

                            let top_title_row = div()
                                .h(px(22.0))
                                .w_full()
                                .flex()
                                .items_center()
                                .when(view_mode == ConflictResolverViewMode::ThreeWay, |d| {
                                    d.child(
                                        div()
                                            .w(col_a_w)
                                            .min_w(px(0.0))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_xs()
                                            .text_color(theme.colors.text_muted)
                                            .whitespace_nowrap()
                                            .child(div().w(px(38.0)).flex_shrink_0())
                                            .child("Base (A, index :1)"),
                                    )
                                    .child(conflict_hsplit_resize_handle(
                                        "conflict_hsplit_handle_first",
                                        ConflictHSplitResizeHandle::First,
                                    ))
                                    .child(
                                        div()
                                            .w(col_b_w)
                                            .min_w(px(0.0))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_xs()
                                            .text_color(theme.colors.text_muted)
                                            .whitespace_nowrap()
                                            .child(div().w(px(38.0)).flex_shrink_0())
                                            .child("Local (B, index :2)"),
                                    )
                                    .child(conflict_hsplit_resize_handle(
                                        "conflict_hsplit_handle_second",
                                        ConflictHSplitResizeHandle::Second,
                                    ))
                                    .child(
                                        div()
                                            .w(col_c_w)
                                            .flex_grow()
                                            .min_w(px(0.0))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_xs()
                                            .text_color(theme.colors.text_muted)
                                            .whitespace_nowrap()
                                            .child(div().w(px(38.0)).flex_shrink_0())
                                            .child("Remote (C, index :3)"),
                                    )
                                })
                                .when(view_mode == ConflictResolverViewMode::TwoWayDiff, |d| {
                                    d.when(mode == ConflictDiffMode::Split, |d| {
                                        let [left_w, right_w] = self.conflict_diff_split_col_widths;
                                        d.child(
                                            div()
                                                .w(left_w)
                                                .min_w(px(0.0))
                                                .px_2()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .text_xs()
                                                .text_color(theme.colors.text_muted)
                                                .whitespace_nowrap()
                                                .child(div().w(px(38.0)).flex_shrink_0())
                                                .child("Local (index :2)"),
                                        )
                                        .child(
                                            div()
                                                .id("conflict_diff_split_resize_handle")
                                                .w(handle_w)
                                                .h_full()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .cursor(CursorStyle::ResizeLeftRight)
                                                .hover(move |s| {
                                                    s.bg(with_alpha(theme.colors.hover, 0.65))
                                                })
                                                .active(move |s| s.bg(theme.colors.active))
                                                .child(
                                                    div()
                                                        .w(px(1.0))
                                                        .h_full()
                                                        .bg(theme.colors.border),
                                                )
                                                .on_drag(
                                                    ConflictDiffSplitResizeHandle::Divider,
                                                    |_, _, _, cx| {
                                                        cx.new(|_| {
                                                            ConflictDiffSplitResizeDragGhost
                                                        })
                                                    },
                                                )
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(
                                                        |this, e: &MouseDownEvent, _w, cx| {
                                                            cx.stop_propagation();
                                                            this.conflict_diff_split_resize = Some(
                                                                ConflictDiffSplitResizeState {
                                                                    start_x: e.position.x,
                                                                    start_ratio: this
                                                                        .conflict_diff_split_ratio,
                                                                },
                                                            );
                                                            cx.notify();
                                                        },
                                                    ),
                                                )
                                                    .on_drag_move(cx.listener(
                                                        |this,
                                                         e: &gpui::DragMoveEvent<
                                                            ConflictDiffSplitResizeHandle,
                                                        >,
                                                         _w,
                                                         cx| {
                                                        let Some(state) =
                                                            this.conflict_diff_split_resize
                                                        else {
                                                            return;
                                                        };
                                                        let Some(new_ratio) =
                                                            next_conflict_diff_split_ratio(
                                                                state,
                                                                e.event.position.x,
                                                                this.conflict_diff_split_col_widths,
                                                            )
                                                        else {
                                                            return;
                                                        };
                                                        if (this.conflict_diff_split_ratio
                                                            - new_ratio)
                                                            .abs()
                                                            <= f32::EPSILON
                                                        {
                                                            return;
                                                        }
                                                        this.conflict_diff_split_ratio = new_ratio;
                                                        cx.notify();
                                                    },
                                                ))
                                                .on_mouse_up(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _e, _w, cx| {
                                                        this.conflict_diff_split_resize = None;
                                                        cx.notify();
                                                    }),
                                                )
                                                .on_mouse_up_out(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _e, _w, cx| {
                                                        this.conflict_diff_split_resize = None;
                                                        cx.notify();
                                                    }),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .w(right_w)
                                                .flex_grow()
                                                .min_w(px(0.0))
                                                .px_2()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .text_xs()
                                                .text_color(theme.colors.text_muted)
                                                .whitespace_nowrap()
                                                .child(div().w(px(38.0)).flex_shrink_0())
                                                .child("Remote (index :3)"),
                                        )
                                    })
                                    .when(mode == ConflictDiffMode::Inline, |d| d)
                                });

                            let top_body: AnyElement = if diff_len == 0 {
                                components::empty_state(theme, "Inputs", "Stage data not available.")
                                    .into_any_element()
                            } else if is_rendered_preview_active {
                                match preview_kind {
                                    Some(RenderedPreviewKind::Svg) => self
                                        .render_conflict_resolver_svg_preview(
                                            theme, &base, &local, &remote,
                                        ),
                                    Some(RenderedPreviewKind::Markdown) => self
                                        .render_conflict_resolver_markdown_preview(theme, cx),
                                    None => components::empty_state(
                                        theme,
                                        "Preview",
                                        "Preview is not available for this file.",
                                    )
                                    .into_any_element(),
                                }
                            } else {
                                let list = match view_mode {
                                    ConflictResolverViewMode::ThreeWay => uniform_list(
                                        "conflict_resolver_three_way_list",
                                        diff_len,
                                        cx.processor(Self::render_conflict_resolver_three_way_rows),
                                    ),
                                    ConflictResolverViewMode::TwoWayDiff => uniform_list(
                                        "conflict_resolver_diff_list",
                                        diff_len,
                                        cx.processor(Self::render_conflict_resolver_diff_rows),
                                    ),
                                }
                                .h_full()
                                .min_h(px(0.0))
                                .with_horizontal_sizing_behavior(
                                    gpui::ListHorizontalSizingBehavior::Unconstrained,
                                )
                                .track_scroll(self.conflict_resolver_diff_scroll.clone());

                                div()
                                    .id("conflict_resolver_diff_scroll")
                                    .relative()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .bg(theme.colors.window_bg)
                                    .child(list)
                                    .child(
                                        components::Scrollbar::new(
                                            "conflict_resolver_diff_scrollbar",
                                            self.conflict_resolver_diff_scroll.clone(),
                                        )
                                        .always_visible()
                                        .render(theme),
                                    )
                                    .into_any_element()
                            };

                            let output_header = div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.colors.text_muted)
                                        .child("Resolved output"),
                                )
                                .child(start_controls);
                            let autosolve_summary =
                                self.conflict_resolver.last_autosolve_summary.clone();

                            // Vertical resize handle between merge inputs and resolved output
                            let vsplit_ratio = self.conflict_resolver_vsplit_ratio;
                            let handle_h = px(PANE_RESIZE_HANDLE_PX);
                            let min_section_h = px(80.0);

                            let vsplit_handle = div()
                                .id("conflict_resolver_vsplit_handle")
                                .w_full()
                                .h(handle_h)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor(CursorStyle::ResizeUpDown)
                                .hover(move |s| s.bg(with_alpha(theme.colors.hover, 0.65)))
                                .active(move |s| s.bg(theme.colors.active))
                                .child(div().h(px(1.0)).w_full().bg(theme.colors.border))
                                .on_drag(
                                    ConflictVSplitResizeHandle::Divider,
                                    |_handle, _offset, _window, cx| {
                                        cx.new(|_cx| ConflictVSplitResizeDragGhost)
                                    },
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                                        cx.stop_propagation();
                                        this.conflict_resolver_vsplit_resize =
                                            Some(ConflictVSplitResizeState {
                                                start_y: e.position.y,
                                                start_ratio: this.conflict_resolver_vsplit_ratio,
                                            });
                                        cx.notify();
                                    }),
                                )
                                .on_drag_move(cx.listener(
                                    move |this,
                                          e: &gpui::DragMoveEvent<ConflictVSplitResizeHandle>,
                                          _w,
                                          cx| {
                                        let Some(state) = this.conflict_resolver_vsplit_resize
                                        else {
                                            return;
                                        };

                                        let total_h = this.last_window_size.height;
                                        // Approximate available height (window - chrome)
                                        let available =
                                            (total_h - px(200.0)).max(min_section_h * 2.0);
                                        let dy = e.event.position.y - state.start_y;
                                        let mut next_top = (available * state.start_ratio) + dy;
                                        next_top = next_top
                                            .max(min_section_h)
                                            .min(available - min_section_h);
                                        this.conflict_resolver_vsplit_ratio =
                                            (next_top / available).clamp(0.1, 0.9);
                                        cx.notify();
                                    },
                                ))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|this, _e, _w, cx| {
                                        this.conflict_resolver_vsplit_resize = None;
                                        cx.notify();
                                    }),
                                )
                                .on_mouse_up_out(
                                    MouseButton::Left,
                                    cx.listener(|this, _e, _w, cx| {
                                        this.conflict_resolver_vsplit_resize = None;
                                        cx.notify();
                                    }),
                                );

                            div()
                                .id("conflict_resolver_panel")
                                .flex()
                                .flex_col()
                                .w_full()
                                .h_full()
                                .min_h(px(0.0))
                                .overflow_hidden()
                                .px_2()
                                .py_2()
                                .gap_1()
                                .child(top_header)
                                .child({
                                    let mut top_section = div()
                                        .min_h(min_section_h)
                                        .border_1()
                                        .border_color(theme.colors.border)
                                        .rounded(px(theme.radii.row))
                                        .overflow_hidden()
                                        .flex()
                                        .flex_col()
                                        .child(top_title_row)
                                        .child(div().border_t_1().border_color(theme.colors.border))
                                        .child(top_body);
                                    top_section.style().flex_grow = Some(vsplit_ratio);
                                    top_section.style().flex_shrink = Some(1.0);
                                    top_section.style().flex_basis = Some(relative(0.).into());
                                    top_section
                                })
                                .child(vsplit_handle)
                                .child(output_header)
                                .when_some(autosolve_summary, |d, summary| {
                                    d.child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.colors.text_muted)
                                            .px_1()
                                            .child(summary),
                                    )
                                })
                                .child({
                                    let mut bottom_section =
                                        div()
                                            .id("conflict_resolver_output")
                                            .min_h(min_section_h)
                                            .border_1()
                                            .border_color(theme.colors.border)
                                            .rounded(px(theme.radii.row))
                                            .overflow_hidden()
                                            .flex()
                                            .flex_col()
                                            .bg(theme.colors.window_bg)
                                            .child(
                                                {
                                                    let outline_len =
                                                        self.conflict_resolved_preview_line_count;
                                                    let outline_list = uniform_list(
                                                        "conflict_resolved_preview_gutter_list",
                                                        outline_len,
                                                        cx.processor(
                                                            Self::render_conflict_resolved_preview_rows,
                                                        ),
                                                    )
                                                    .h_full()
                                                    .min_h(px(0.0))
                                                    .track_scroll(
                                                        self.conflict_resolved_preview_scroll.clone(),
                                                    );

                                                    div()
                                                        .id("conflict_resolver_output_body")
                                                        .relative()
                                                        .flex_1()
                                                        .min_h(px(0.0))
                                                        .bg(theme.colors.window_bg)
                                                        .child(
                                                            div()
                                                                .id("conflict_resolver_output_surface")
                                                                .h_full()
                                                                .min_h(px(0.0))
                                                                .p_2()
                                                                .font_family("monospace")
                                                                .on_mouse_down(
                                                                    MouseButton::Right,
                                                                    cx.listener(
                                                                        |this,
                                                                         e: &MouseDownEvent,
                                                                         window,
                                                                         cx| {
                                                                            this.open_conflict_resolver_output_context_menu(
                                                                                e.position,
                                                                                window,
                                                                                cx,
                                                                            );
                                                                        },
                                                                    ),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .flex()
                                                                        .items_start()
                                                                        .h_full()
                                                                        .min_h(px(0.0))
                                                                        .min_w_full()
                                                                        .child(
                                                                            div()
                                                                                .id("conflict_resolver_output_gutter")
                                                                                .w(px(92.0))
                                                                                .h_full()
                                                                                .min_h(px(0.0))
                                                                                .flex_shrink_0()
                                                                                .border_r_1()
                                                                                .border_color(
                                                                                    theme.colors.border,
                                                                                )
                                                                                .child(outline_list),
                                                                        )
                                                                        .child(
                                                                            div()
                                                                                .id(
                                                                                    "conflict_resolver_output_editor",
                                                                                )
                                                                                .relative()
                                                                                .flex_1()
                                                                                .min_w(px(0.0))
                                                                                .h_full()
                                                                                .min_h(px(0.0))
                                                                                .pl_2()
                                                                                .child(
                                                                                    uniform_list(
                                                                                        "conflict_resolved_output_list",
                                                                                        outline_len,
                                                                                        cx.processor(
                                                                                            Self::render_conflict_resolved_output_rows,
                                                                                        ),
                                                                                    )
                                                                                    .h_full()
                                                                                    .min_h(px(0.0))
                                                                                    .track_scroll(
                                                                                        self.conflict_resolved_preview_scroll.clone(),
                                                                                    )
                                                                                    .with_horizontal_sizing_behavior(
                                                                                        gpui::ListHorizontalSizingBehavior::Unconstrained,
                                                                                    )
                                                                                    .into_any_element(),
                                                                                ),
                                                                        ),
                                                                ),
                                                        )
                                                        .child(
                                                            components::Scrollbar::new(
                                                                "conflict_resolver_output_scrollbar",
                                                                self.conflict_resolved_preview_scroll
                                                                    .clone(),
                                                            )
                                                            .always_visible()
                                                            .render(theme),
                                                        )
                                                },
                                            );
                                    bottom_section.style().flex_grow = Some(1.0 - vsplit_ratio);
                                    bottom_section.style().flex_shrink = Some(1.0);
                                    bottom_section.style().flex_basis = Some(relative(0.).into());
                                    bottom_section
                                })
                                .into_any_element()
                            }
                        }
                    } else {
                        debug_assert!(false, "conflict resolver rendered without active repo id");
                        components::empty_state(theme, title, "Repository context unavailable.")
                            .into_any_element()
                    }
                }
            }
        } else if is_conflict_compare {
            match (repo, conflict_target_path) {
                (None, _) => {
                    components::empty_state(theme, "Resolve", "No repository.").into_any_element()
                }
                (_, None) => {
                    components::empty_state(theme, "Resolve", "No conflicted file selected.")
                        .into_any_element()
                }
                (Some(repo), Some(path)) => {
                    let title: SharedString =
                        format!("Resolve conflict: {}", self.cached_path_display(&path)).into();

                    match &repo.conflict_state.conflict_file {
                        Loadable::NotLoaded | Loadable::Loading => {
                            components::empty_state(theme, title, "Loading conflict data…")
                                .into_any_element()
                        }
                        Loadable::Error(e) => {
                            components::empty_state(theme, title, e.clone()).into_any_element()
                        }
                        Loadable::Ready(None) => {
                            components::empty_state(theme, title, "No conflict data.")
                                .into_any_element()
                        }
                        Loadable::Ready(Some(file)) => {
                            if file.path != path {
                                components::empty_state(theme, title, "Loading conflict data…")
                                    .into_any_element()
                            } else {
                                let ours_label: SharedString = if file.ours.is_some() {
                                    "Ours".into()
                                } else {
                                    "Ours (deleted)".into()
                                };
                                let theirs_label: SharedString = if file.theirs.is_some() {
                                    "Theirs".into()
                                } else {
                                    "Theirs (deleted)".into()
                                };

                                let columns_header = components::split_columns_header(
                                    theme,
                                    ours_label,
                                    theirs_label,
                                );

                                let diff_len = match self.diff_view {
                                    DiffViewMode::Split => self.conflict_resolver.diff_rows.len(),
                                    DiffViewMode::Inline => {
                                        self.conflict_resolver.inline_rows.len()
                                    }
                                };

                                let diff_body: AnyElement = if diff_len == 0 {
                                    components::empty_state(
                                        theme,
                                        "Diff",
                                        "No conflict diff to show.",
                                    )
                                    .into_any_element()
                                } else {
                                    let scroll_handle =
                                        self.diff_scroll.0.borrow().base_handle.clone();
                                    let list = uniform_list(
                                        "conflict_compare_diff",
                                        diff_len,
                                        cx.processor(Self::render_conflict_compare_diff_rows),
                                    )
                                    .h_full()
                                    .min_h(px(0.0))
                                    .track_scroll(self.diff_scroll.clone())
                                    .with_horizontal_sizing_behavior(
                                        gpui::ListHorizontalSizingBehavior::Unconstrained,
                                    );

                                    div()
                                        .id("conflict_compare_container")
                                        .relative()
                                        .flex()
                                        .flex_col()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .bg(theme.colors.window_bg)
                                        .child(columns_header)
                                        .child(
                                            div()
                                                .id("conflict_compare_scroll_container")
                                                .relative()
                                                .flex_1()
                                                .min_h(px(0.0))
                                                .child(list)
                                                .child(
                                                    components::Scrollbar::new(
                                                        "conflict_compare_scrollbar",
                                                        self.diff_scroll.clone(),
                                                    )
                                                    .always_visible()
                                                    .render(theme),
                                                )
                                                .child(
                                                    components::Scrollbar::horizontal(
                                                        "conflict_compare_hscrollbar",
                                                        scroll_handle,
                                                    )
                                                    .always_visible()
                                                    .render(theme),
                                                ),
                                        )
                                        .into_any_element()
                                };

                                diff_body
                            }
                        }
                    }
                }
            }
        } else if wants_file_diff {
            self.render_selected_file_diff(theme, cx)
        } else {
            match repo {
                None => components::empty_state(theme, "Diff", "No repository.").into_any_element(),
                Some(repo) => match &repo.diff_state.diff {
                    Loadable::NotLoaded => {
                        components::empty_state(theme, "Diff", "Select a file.").into_any_element()
                    }
                    Loadable::Loading => {
                        components::empty_state(theme, "Diff", "Loading").into_any_element()
                    }
                    Loadable::Error(e) => {
                        self.diff_raw_input.update(cx, |input, cx| {
                            input.set_theme(theme, cx);
                            input.set_text(e.clone(), cx);
                            input.set_read_only(true, cx);
                        });
                        div()
                            .id("diff_error_scroll")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .child(self.diff_raw_input.clone())
                            .into_any_element()
                    }
                    Loadable::Ready(diff) => {
                        if wants_file_diff {
                            self.render_selected_file_diff(theme, cx)
                        } else {
                            if self.diff_word_wrap {
                                let approx_len: usize = diff
                                    .lines
                                    .iter()
                                    .map(|l| l.text.len().saturating_add(1))
                                    .sum();
                                let mut raw = String::with_capacity(approx_len);
                                for line in &diff.lines {
                                    raw.push_str(line.text.as_ref());
                                    raw.push('\n');
                                }
                                self.diff_raw_input.update(cx, |input, cx| {
                                    input.set_theme(theme, cx);
                                    input.set_soft_wrap(true, cx);
                                    input.set_text(raw, cx);
                                    input.set_read_only(true, cx);
                                });
                                div()
                                    .id("diff_word_wrap_scroll")
                                    .bg(theme.colors.window_bg)
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .overflow_y_scroll()
                                    .child(self.diff_raw_input.clone())
                                    .into_any_element()
                            } else {
                                if self.diff_cache_repo_id != Some(repo.id)
                                    || self.diff_cache_rev != repo.diff_state.diff_rev
                                    || self.diff_cache_target != repo.diff_state.diff_target
                                    || self.patch_diff_row_len() != diff.lines.len()
                                {
                                    self.rebuild_diff_cache(cx);
                                }

                                self.ensure_diff_visible_indices();
                                self.maybe_autoscroll_diff_to_first_change();
                                if self.patch_diff_row_len() == 0 {
                                    components::empty_state(theme, "Diff", "No differences.")
                                        .into_any_element()
                                } else if self.diff_visible_len() == 0 {
                                    components::empty_state(theme, "Diff", "Nothing to render.")
                                        .into_any_element()
                                } else {
                                    let scroll_handle =
                                        self.diff_scroll.0.borrow().base_handle.clone();
                                    let markers = self.diff_scrollbar_markers_cache.clone();
                                    match self.diff_view {
                                        DiffViewMode::Inline => {
                                            let list = uniform_list(
                                                "diff",
                                                self.diff_visible_len(),
                                                cx.processor(Self::render_diff_rows),
                                            )
                                            .h_full()
                                            .min_h(px(0.0))
                                            .track_scroll(self.diff_scroll.clone())
                                            .with_horizontal_sizing_behavior(
                                                gpui::ListHorizontalSizingBehavior::Unconstrained,
                                            );
                                            div()
                                                .id("diff_scroll_container")
                                                .relative()
                                                .h_full()
                                                .min_h(px(0.0))
                                                .bg(theme.colors.window_bg)
                                                .child(list)
                                                .child(
                                                    components::Scrollbar::new(
                                                        "diff_scrollbar",
                                                        self.diff_scroll.clone(),
                                                    )
                                                    .markers(markers)
                                                    .always_visible()
                                                    .render(theme),
                                                )
                                                .child(
                                                    components::Scrollbar::horizontal(
                                                        "diff_hscrollbar",
                                                        scroll_handle,
                                                    )
                                                    .always_visible()
                                                    .render(theme),
                                                )
                                                .into_any_element()
                                        }
                                        DiffViewMode::Split => {
                                            self.sync_diff_split_vertical_scroll();
                                            let right_scroll_handle = self
                                                .diff_split_right_scroll
                                                .0
                                                .borrow()
                                                .base_handle
                                                .clone();
                                            let count = self.diff_visible_len();
                                            let left = uniform_list(
                                                "diff_split_left",
                                                count,
                                                cx.processor(Self::render_diff_split_left_rows),
                                            )
                                            .h_full()
                                            .min_h(px(0.0))
                                            .track_scroll(self.diff_scroll.clone())
                                            .with_horizontal_sizing_behavior(
                                                gpui::ListHorizontalSizingBehavior::Unconstrained,
                                            );
                                            let right = uniform_list(
                                                "diff_split_right",
                                                count,
                                                cx.processor(Self::render_diff_split_right_rows),
                                            )
                                            .h_full()
                                            .min_h(px(0.0))
                                            .track_scroll(self.diff_split_right_scroll.clone())
                                            .with_horizontal_sizing_behavior(
                                                gpui::ListHorizontalSizingBehavior::Unconstrained,
                                            );

                                            let handle_w = px(PANE_RESIZE_HANDLE_PX);
                                            let min_col_w = px(DIFF_SPLIT_COL_MIN_PX);
                                            let main_w = self.main_pane_content_width(cx);
                                            let available = (main_w - handle_w).max(px(0.0));
                                            let left_w = if available <= min_col_w * 2.0 {
                                                available * 0.5
                                            } else {
                                                (available * self.diff_split_ratio)
                                                    .max(min_col_w)
                                                    .min(available - min_col_w)
                                            };
                                            let right_w = available - left_w;

                                            let resize_handle = |id: &'static str| {
                                                div()
                                                    .id(id)
                                                    .w(handle_w)
                                                    .h_full()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .cursor(CursorStyle::ResizeLeftRight)
                                                    .hover(move |s| {
                                                        s.bg(with_alpha(theme.colors.hover, 0.65))
                                                    })
                                                    .active(move |s| s.bg(theme.colors.active))
                                                    .child(
                                                        div()
                                                            .w(px(1.0))
                                                            .h_full()
                                                            .bg(theme.colors.border),
                                                    )
                                                    .on_drag(
                                                        DiffSplitResizeHandle::Divider,
                                                        |_handle, _offset, _window, cx| {
                                                            cx.new(|_cx| DiffSplitResizeDragGhost)
                                                        },
                                                    )
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(
                                                            move |this,
                                                                  e: &MouseDownEvent,
                                                                  _w,
                                                                  cx| {
                                                                cx.stop_propagation();
                                                                this.diff_split_resize = Some(
                                                                    DiffSplitResizeState {
                                                                        handle:
                                                                            DiffSplitResizeHandle::Divider,
                                                                        start_x: e.position.x,
                                                                        start_ratio: this
                                                                            .diff_split_ratio,
                                                                    },
                                                                );
                                                                cx.notify();
                                                            },
                                                        ),
                                                    )
                                                    .on_drag_move(cx.listener(
                                                        move |this,
                                                              e: &gpui::DragMoveEvent<
                                                            DiffSplitResizeHandle,
                                                        >,
                                                              _w,
                                                              cx| {
                                                            let Some(state) = this.diff_split_resize
                                                            else {
                                                                return;
                                                            };
                                                            if state.handle != *e.drag(cx) {
                                                                return;
                                                            }

                                                            let main_w = this
                                                                .main_pane_content_width(cx);
                                                            let available =
                                                                (main_w - handle_w).max(px(0.0));
                                                            if available <= min_col_w * 2.0 {
                                                                this.diff_split_ratio = 0.5;
                                                                cx.notify();
                                                                return;
                                                            }

                                                            let dx =
                                                                e.event.position.x - state.start_x;
                                                            let max_left = available - min_col_w;
                                                            let mut next_left = (available
                                                                * state.start_ratio)
                                                                + dx;
                                                            next_left = next_left
                                                                .max(min_col_w)
                                                                .min(max_left);

                                                            this.diff_split_ratio =
                                                                (next_left / available)
                                                                    .clamp(0.0, 1.0);
                                                            cx.notify();
                                                        },
                                                    ))
                                                    .on_mouse_up(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _e, _w, cx| {
                                                            this.diff_split_resize = None;
                                                            cx.notify();
                                                        }),
                                                    )
                                                    .on_mouse_up_out(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _e, _w, cx| {
                                                            this.diff_split_resize = None;
                                                            cx.notify();
                                                        }),
                                                    )
                                            };

                                            let columns_header = div()
                                                .id("diff_split_columns_header")
                                                .h(px(components::CONTROL_HEIGHT_PX))
                                                .flex()
                                                .items_center()
                                                .text_xs()
                                                .text_color(theme.colors.text_muted)
                                                .bg(theme.colors.surface_bg_elevated)
                                                .border_b_1()
                                                .border_color(theme.colors.border)
                                                .child(
                                                    div()
                                                        .w(left_w)
                                                        .min_w(px(0.0))
                                                        .px_2()
                                                        .overflow_hidden()
                                                        .whitespace_nowrap()
                                                        .child("A (local / before)"),
                                                )
                                                .child(resize_handle(
                                                    "diff_split_resize_handle_header",
                                                ))
                                                .child(
                                                    div()
                                                        .w(right_w)
                                                        .min_w(px(0.0))
                                                        .px_2()
                                                        .overflow_hidden()
                                                        .whitespace_nowrap()
                                                        .child("B (remote / after)"),
                                                );

                                            div()
                                                .id("diff_split_scroll_container")
                                                .relative()
                                                .h_full()
                                                .min_h(px(0.0))
                                                .flex()
                                                .flex_col()
                                                .bg(theme.colors.window_bg)
                                                .child(columns_header)
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_h(px(0.0))
                                                        .flex()
                                                        .child(
                                                            div()
                                                                .relative()
                                                                .w(left_w)
                                                                .min_w(px(0.0))
                                                                .h_full()
                                                                .child(left)
                                                                .child(
                                                                    components::Scrollbar::horizontal(
                                                                        "diff_split_left_hscrollbar",
                                                                        scroll_handle.clone(),
                                                                    )
                                                                    .always_visible()
                                                                    .render(theme),
                                                                ),
                                                        )
                                                        .child(resize_handle(
                                                            "diff_split_resize_handle_body",
                                                        ))
                                                        .child(
                                                            div()
                                                                .relative()
                                                                .w(right_w)
                                                                .min_w(px(0.0))
                                                                .h_full()
                                                                .child(right)
                                                                .child(
                                                                    components::Scrollbar::horizontal(
                                                                        "diff_split_right_hscrollbar",
                                                                        right_scroll_handle,
                                                                    )
                                                                    .always_visible()
                                                                    .render(theme),
                                                                ),
                                                        ),
                                                )
                                                .child(
                                                    components::Scrollbar::new(
                                                        "diff_scrollbar",
                                                        self.diff_scroll.clone(),
                                                    )
                                                    .markers(markers)
                                                    .always_visible()
                                                    .render(theme),
                                                )
                                                .into_any_element()
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        };
        self.diff_text_layout_cache_epoch = self.diff_text_layout_cache_epoch.wrapping_add(1);
        self.prune_diff_text_layout_cache();
        self.diff_text_hitboxes.clear();
        let diff_editor_menu_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == "diff_editor_menu");

        div()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .h_full()
            .min_h(px(0.0))
            .bg(theme.colors.surface_bg_elevated)
            .when(diff_editor_menu_active, |d| d.bg(theme.colors.active))
            .track_focus(&self.diff_panel_focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, _cx| {
                    window.focus(&this.diff_panel_focus_handle);
                }),
            )
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                let key = e.keystroke.key.as_str();
                let mods = e.keystroke.modifiers;

                let mut handled = false;

                if key == "escape" && !mods.control && !mods.alt && !mods.platform && !mods.function
                {
                    if this.diff_search_active {
                        this.diff_search_active = false;
                        this.diff_search_matches.clear();
                        this.diff_search_match_ix = None;
                        this.clear_diff_text_query_overlay_cache();
                        this.clear_worktree_preview_segments_cache();
                        this.clear_conflict_diff_query_overlay_caches();
                        window.focus(&this.diff_panel_focus_handle);
                        handled = true;
                    }
                    if !handled && let Some(repo_id) = this.active_repo_id() {
                        this.clear_status_multi_selection(repo_id, cx);
                        this.clear_diff_selection_or_exit(repo_id, cx);
                        handled = true;
                    }
                }

                if !handled
                    && (mods.control || mods.platform)
                    && !mods.alt
                    && !mods.function
                    && key == "f"
                {
                    this.activate_diff_search(window, cx);
                    handled = true;
                }

                if !handled
                    && this.diff_search_active
                    && matches!(key, "f2" | "f3")
                    && !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                {
                    if key == "f2" {
                        this.diff_search_prev_match();
                    } else {
                        this.diff_search_next_match();
                    }
                    handled = true;
                }

                if !handled
                    && key == "space"
                    && !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                    && !this
                        .diff_raw_input
                        .read(cx)
                        .focus_handle()
                        .is_focused(window)
                    && let Some(repo_id) = this.active_repo_id()
                    && let Some(repo) = this.active_repo()
                    && let Some(DiffTarget::WorkingTree { path, area }) =
                        repo.diff_state.diff_target.clone()
                {
                    let next_path_in_area = |entries: &[gitcomet_core::domain::FileStatus]| {
                        if entries.len() <= 1 {
                            return None;
                        }

                        let (prev_ix, next_ix) =
                            Self::status_prev_next_indices(entries, path.as_path());
                        next_ix
                            .or(prev_ix)
                            .and_then(|ix| entries.get(ix).map(|e| e.path.clone()))
                    };

                    match (&repo.status, area) {
                        (Loadable::Ready(status), DiffArea::Unstaged) => {
                            this.store.dispatch(Msg::StagePath {
                                repo_id,
                                path: path.clone(),
                            });
                            if let Some(next_path) = next_path_in_area(&status.unstaged) {
                                this.store.dispatch(Msg::SelectDiff {
                                    repo_id,
                                    target: DiffTarget::WorkingTree {
                                        path: next_path,
                                        area: DiffArea::Unstaged,
                                    },
                                });
                            } else {
                                this.clear_diff_selection_or_exit(repo_id, cx);
                            }
                        }
                        (Loadable::Ready(status), DiffArea::Staged) => {
                            this.store.dispatch(Msg::UnstagePath {
                                repo_id,
                                path: path.clone(),
                            });
                            if let Some(next_path) = next_path_in_area(&status.staged) {
                                this.store.dispatch(Msg::SelectDiff {
                                    repo_id,
                                    target: DiffTarget::WorkingTree {
                                        path: next_path,
                                        area: DiffArea::Staged,
                                    },
                                });
                            } else {
                                this.clear_diff_selection_or_exit(repo_id, cx);
                            }
                        }
                        (_, DiffArea::Unstaged) => {
                            this.store.dispatch(Msg::StagePath {
                                repo_id,
                                path: path.clone(),
                            });
                        }
                        (_, DiffArea::Staged) => {
                            this.store.dispatch(Msg::UnstagePath {
                                repo_id,
                                path: path.clone(),
                            });
                        }
                    }
                    this.rebuild_diff_cache(cx);
                    handled = true;
                }

                if !handled
                    && (key == "f1" || key == "f4")
                    && !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                    && let Some(repo_id) = this.active_repo_id()
                {
                    let direction = if key == "f1" { -1 } else { 1 };
                    handled = this.try_select_adjacent_status_file(repo_id, direction, window, cx);
                }

                let copy_target_is_focused = this
                    .diff_raw_input
                    .read(cx)
                    .focus_handle()
                    .is_focused(window);
                let is_file_preview = this.is_file_preview_active();
                if is_file_preview {
                    if !handled
                        && !copy_target_is_focused
                        && (mods.control || mods.platform)
                        && !mods.alt
                        && !mods.function
                        && key == "c"
                        && this.diff_text_has_selection()
                    {
                        this.copy_selected_diff_text_to_clipboard(cx);
                        handled = true;
                    }

                    if !handled
                        && !copy_target_is_focused
                        && (mods.control || mods.platform)
                        && !mods.alt
                        && !mods.function
                        && key == "a"
                    {
                        this.select_all_diff_text();
                        handled = true;
                    }

                    if handled {
                        cx.stop_propagation();
                        cx.notify();
                    }
                    return;
                }

                let conflict_resolver_active = this.is_conflict_resolver_active();
                let markdown_preview_active = this.is_markdown_preview_active();
                let conflict_preview_active = this.is_conflict_rendered_preview_active();

                if mods.alt && !mods.control && !mods.platform && !mods.function {
                    match key {
                        "i" | "s" => {
                            let (conflict_mode, diff_mode) = if key == "i" {
                                (ConflictDiffMode::Inline, DiffViewMode::Inline)
                            } else {
                                (ConflictDiffMode::Split, DiffViewMode::Split)
                            };
                            if conflict_resolver_active {
                                if !conflict_preview_active {
                                    this.conflict_resolver_set_mode(conflict_mode, cx);
                                }
                            } else if !markdown_preview_active {
                                this.diff_view = diff_mode;
                                this.clear_diff_text_style_caches();
                            }
                            handled = true;
                        }
                        "h" => {
                            let is_file_preview = this.is_file_preview_active();
                            let wants_file_diff = !is_file_preview
                                && !this.is_worktree_target_directory()
                                && this.active_repo().is_some_and(|r| {
                                    Self::is_file_diff_target(r.diff_state.diff_target.as_ref())
                                });
                            if !is_file_preview && !wants_file_diff {
                                this.open_popover_at_cursor(PopoverKind::DiffHunks, window, cx);
                                handled = true;
                            }
                        }
                        "w" if !markdown_preview_active && !conflict_preview_active => {
                            this.toggle_show_whitespace();
                            handled = true;
                        }
                        "up" if !markdown_preview_active => {
                            this.diff_jump_prev();
                            handled = true;
                        }
                        "down" if !markdown_preview_active => {
                            this.diff_jump_next();
                            handled = true;
                        }
                        _ => {}
                    }
                }

                if !handled
                    && matches!(key, "f2" | "f3" | "f7")
                    && !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                {
                    if let Some(direction) =
                        conflict_resolver::conflict_nav_direction_for_key(key, mods.shift)
                    {
                        if conflict_resolver_active {
                            if !conflict_preview_active {
                                match direction {
                                    conflict_resolver::ConflictNavDirection::Prev => {
                                        this.conflict_jump_prev(cx);
                                    }
                                    conflict_resolver::ConflictNavDirection::Next => {
                                        this.conflict_jump_next(cx);
                                    }
                                }
                            }
                        } else if !markdown_preview_active {
                            match direction {
                                conflict_resolver::ConflictNavDirection::Prev => {
                                    this.diff_jump_prev()
                                }
                                conflict_resolver::ConflictNavDirection::Next => {
                                    this.diff_jump_next()
                                }
                            }
                        }
                    }
                    handled = true;
                }

                // A/B/C/D quick-pick for conflict resolver.
                if !handled
                    && conflict_resolver_active
                    && !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                    && !copy_target_is_focused
                    && !this
                        .conflict_resolver_input
                        .read(cx)
                        .focus_handle()
                        .is_focused(window)
                    && this.conflict_resolver_conflict_count() > 0
                    && let Some(choice) = conflict_resolver::conflict_quick_pick_choice_for_key(key)
                {
                    this.conflict_resolver_pick_active_conflict(choice, cx);
                    handled = true;
                }

                if !handled
                    && !copy_target_is_focused
                    && (mods.control || mods.platform)
                    && !mods.alt
                    && !mods.function
                    && key == "c"
                    && this.diff_text_has_selection()
                {
                    this.copy_selected_diff_text_to_clipboard(cx);
                    handled = true;
                }

                if !handled
                    && !copy_target_is_focused
                    && (mods.control || mods.platform)
                    && !mods.alt
                    && !mods.function
                    && key == "a"
                {
                    this.select_all_diff_text();
                    handled = true;
                }

                if handled {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                header
                    .h(px(components::CONTROL_HEIGHT_MD_PX))
                    .px_2()
                    .bg(theme.colors.surface_bg_elevated)
                    .border_b_1()
                    .border_color(theme.colors.border),
            )
            .child(div().flex_1().min_h(px(0.0)).w_full().h_full().child(body))
            .child(DiffTextSelectionTracker { view: cx.entity() })
    }

    fn render_conflict_resolver_svg_preview(
        &self,
        theme: AppTheme,
        base: &str,
        local: &str,
        remote: &str,
    ) -> AnyElement {
        let svg_image = |text: &str| -> Option<Arc<gpui::Image>> {
            if text.is_empty() {
                return None;
            }
            rasterize_svg_preview_image(text.as_bytes()).or_else(|| {
                Some(Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Svg,
                    text.as_bytes().to_vec(),
                )))
            })
        };
        let base_img = svg_image(base);
        let ours_img = svg_image(local);
        let theirs_img = svg_image(remote);

        let preview_cell =
            |id: &'static str, label: &'static str, img: Option<Arc<gpui::Image>>| {
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
                            .child(match img {
                                Some(data) => gpui::img(data)
                                    .w_full()
                                    .h_full()
                                    .object_fit(gpui::ObjectFit::Contain)
                                    .into_any_element(),
                                None => div()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child("(empty)")
                                    .into_any_element(),
                            }),
                    )
            };

        div()
            .id("conflict_resolver_preview")
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .flex()
            .gap_2()
            .p_2()
            .bg(theme.colors.window_bg)
            .child(preview_cell("conflict_preview_base", "Base (A)", base_img))
            .child(preview_cell("conflict_preview_ours", "Local (B)", ours_img))
            .child(preview_cell(
                "conflict_preview_theirs",
                "Remote (C)",
                theirs_img,
            ))
            .into_any_element()
    }

    fn render_conflict_resolver_markdown_preview(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        self.ensure_conflict_markdown_preview_cache();
        self.sync_conflict_preview_vertical_scroll();

        let scroll_for = |side: ThreeWayColumn| -> ScrollHandle {
            match side {
                ThreeWayColumn::Base => &self.conflict_resolver_diff_scroll,
                ThreeWayColumn::Ours => &self.conflict_preview_ours_scroll,
                ThreeWayColumn::Theirs => &self.conflict_preview_theirs_scroll,
            }
            .0
            .borrow()
            .base_handle
            .clone()
        };

        let row_count = |side: ThreeWayColumn| -> usize {
            match self.conflict_resolver.markdown_preview.document(side) {
                Loadable::Ready(doc) => doc.rows.len(),
                _ => 0,
            }
        };
        let tallest = [
            ThreeWayColumn::Base,
            ThreeWayColumn::Ours,
            ThreeWayColumn::Theirs,
        ]
        .into_iter()
        .max_by_key(|s| row_count(*s))
        .unwrap_or(ThreeWayColumn::Base);
        let vertical_handle = scroll_for(tallest);

        div()
            .id("conflict_resolver_preview")
            .relative()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .flex()
            .gap_2()
            .p_2()
            .bg(theme.colors.window_bg)
            .child(self.render_conflict_markdown_preview_column(theme, ThreeWayColumn::Base, cx))
            .child(self.render_conflict_markdown_preview_column(theme, ThreeWayColumn::Ours, cx))
            .child(self.render_conflict_markdown_preview_column(theme, ThreeWayColumn::Theirs, cx))
            .child(
                components::Scrollbar::new("conflict_markdown_preview_scrollbar", vertical_handle)
                    .always_visible()
                    .render(theme),
            )
            .into_any_element()
    }

    fn render_conflict_markdown_preview_column(
        &mut self,
        theme: AppTheme,
        side: ThreeWayColumn,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let (id, list_id, hscrollbar_id, label, scroll) = match side {
            ThreeWayColumn::Base => (
                "conflict_preview_base",
                "conflict_preview_base_list",
                "conflict_preview_base_hscrollbar",
                "Base (A)",
                self.conflict_resolver_diff_scroll.clone(),
            ),
            ThreeWayColumn::Ours => (
                "conflict_preview_ours",
                "conflict_preview_ours_list",
                "conflict_preview_ours_hscrollbar",
                "Local (B)",
                self.conflict_preview_ours_scroll.clone(),
            ),
            ThreeWayColumn::Theirs => (
                "conflict_preview_theirs",
                "conflict_preview_theirs_list",
                "conflict_preview_theirs_hscrollbar",
                "Remote (C)",
                self.conflict_preview_theirs_scroll.clone(),
            ),
        };
        let status = |message: SharedString| {
            div()
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .items_center()
                .justify_center()
                .p_2()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(message)
                .into_any_element()
        };

        // Macro to build the column list+scrollbar from a side-specific processor.
        // Each side needs its own fn item type for `cx.processor()`.
        macro_rules! mk_list {
            ($document:expr, $processor:expr) => {{
                let scroll_handle = scroll.0.borrow().base_handle.clone();
                let list = uniform_list(list_id, $document.rows.len(), cx.processor($processor))
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(scroll)
                    .with_horizontal_sizing_behavior(
                        gpui::ListHorizontalSizingBehavior::Unconstrained,
                    );
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(list)
                    .child(
                        components::Scrollbar::horizontal(hscrollbar_id, scroll_handle)
                            .always_visible()
                            .render(theme),
                    )
                    .into_any_element()
            }};
        }

        let body = match (side, self.conflict_resolver.markdown_preview.document(side)) {
            (_, Loadable::NotLoaded | Loadable::Loading) => status("Processing preview…".into()),
            (_, Loadable::Error(error)) => status(error.clone().into()),
            (_, Loadable::Ready(document)) if document.rows.is_empty() => {
                status("Empty file.".into())
            }
            (ThreeWayColumn::Base, Loadable::Ready(doc)) => {
                mk_list!(doc, Self::render_conflict_markdown_base_rows)
            }
            (ThreeWayColumn::Ours, Loadable::Ready(doc)) => {
                mk_list!(doc, Self::render_conflict_markdown_ours_rows)
            }
            (ThreeWayColumn::Theirs, Loadable::Ready(doc)) => {
                mk_list!(doc, Self::render_conflict_markdown_theirs_rows)
            }
        };

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
                    .child(body),
            )
            .into_any_element()
    }
}
