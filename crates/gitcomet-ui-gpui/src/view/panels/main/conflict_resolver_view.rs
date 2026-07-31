//! Conflict resolver rendering for [`MainPaneView`].
//!
//! Extracted from `diff_view.rs`: the kdiff3-style three-way resolver pane,
//! its toolbar control clusters, and the rendered (SVG/Markdown) conflict
//! previews. See UI_DESIGN.md section 30 for the design spec.

use super::*;

pub(super) use conflict_resolver::CONFLICT_BOTTOM_OVERSCROLL_ROWS;

fn conflict_output_wheel_requires_notify(delta_y: Pixels, horizontal_changed: bool) -> bool {
    delta_y != px(0.0) || horizontal_changed
}

fn conflict_output_post_layout_scroll_y(gutter_y: Pixels, editor_max_y: Pixels) -> Pixels {
    gutter_y.clamp(-editor_max_y.max(px(0.0)), px(0.0))
}

#[cfg(test)]
mod wheel_tests {
    use super::*;

    #[test]
    fn resolved_output_vertical_wheel_keeps_gutter_sync_render_scheduled() {
        assert!(conflict_output_wheel_requires_notify(px(-1.0), false));
        assert!(conflict_output_wheel_requires_notify(px(0.0), true));
        assert!(!conflict_output_wheel_requires_notify(px(0.0), false));
    }

    #[test]
    fn resolved_output_post_layout_scroll_clamps_to_editor_range() {
        assert_eq!(
            conflict_output_post_layout_scroll_y(px(-240.0), px(180.0)),
            px(-180.0)
        );
        assert_eq!(
            conflict_output_post_layout_scroll_y(px(-120.0), px(180.0)),
            px(-120.0)
        );
        assert_eq!(
            conflict_output_post_layout_scroll_y(px(12.0), px(180.0)),
            px(0.0)
        );
    }
}

impl MainPaneView {
    /// Toolbar controls for simple conflict strategies (binary, keep/delete,
    /// decision-only): file navigation plus resolved counts.
    pub(super) fn conflict_toolbar_simple_controls(
        &self,
        mut controls: gpui::Div,
        prev_file_btn: Option<AnyElement>,
        next_file_btn: Option<AnyElement>,
        theme: AppTheme,
    ) -> gpui::Div {
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
        controls
    }

    /// Toolbar controls for the full text resolver: conflict navigation,
    /// pick actions, view-mode toggles, autosolve entry points, and the
    /// save/stage completion actions.
    pub(super) fn conflict_toolbar_full_controls(
        &self,
        mut controls: gpui::Div,
        prev_file_btn: Option<AnyElement>,
        next_file_btn: Option<AnyElement>,
        conflict_rendered_preview_active: bool,
        repo_id: Option<RepoId>,
        conflict_target_path: &Option<std::path::PathBuf>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        controls = controls
            .when_some(prev_file_btn, |d, btn| d.child(btn))
            .when(!conflict_rendered_preview_active, |d| {
                let can_nav_prev = self.conflict_has_prev();
                let can_nav_next = self.conflict_has_next();
                let can_jump_first = self.conflict_has_prev_delta();
                let can_jump_last = self.conflict_has_next_delta();
                let can_prev_unresolved = self.conflict_has_prev_unresolved();
                let can_next_unresolved = self.conflict_has_next_unresolved();

                d.child(
                    components::Button::new("conflict_first", "")
                        .start_slot(svg_icon(
                            "icons/arrow_up_to_line.svg",
                            theme.colors.text,
                            px(14.0),
                        ))
                        .style(components::ButtonStyle::Outlined)
                        .borderless()
                        .disabled(!can_jump_first)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.conflict_jump_first(cx);
                        })
                        .gitcomet_tooltip(theme, "First delta (Ctrl+Home)".into()),
                )
                .child(
                    components::Button::new("conflict_prev", "")
                        .start_slot(svg_icon("icons/arrow_up.svg", theme.colors.text, px(14.0)))
                        .style(components::ButtonStyle::Outlined)
                        .borderless()
                        .disabled(!can_nav_prev)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.conflict_jump_prev(cx);
                            cx.notify();
                        })
                        .gitcomet_tooltip(
                            theme,
                            format!(
                                "Previous conflict (F2 / Shift+F7 / {})",
                                crate::view::shortcut_labels::alt_shortcut("Up")
                            )
                            .into(),
                        ),
                )
                .child(
                    components::Button::new("conflict_next", "")
                        .start_slot(svg_icon(
                            "icons/arrow_down.svg",
                            theme.colors.text,
                            px(14.0),
                        ))
                        .style(components::ButtonStyle::Outlined)
                        .borderless()
                        .disabled(!can_nav_next)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.conflict_jump_next(cx);
                            cx.notify();
                        })
                        .gitcomet_tooltip(
                            theme,
                            format!(
                                "Next conflict (F3 / F7 / {})",
                                crate::view::shortcut_labels::alt_shortcut("Down")
                            )
                            .into(),
                        ),
                )
                .child(
                    components::Button::new("conflict_last", "")
                        .start_slot(svg_icon(
                            "icons/arrow_down_to_line.svg",
                            theme.colors.text,
                            px(14.0),
                        ))
                        .style(components::ButtonStyle::Outlined)
                        .borderless()
                        .disabled(!can_jump_last)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.conflict_jump_last(cx);
                        })
                        .gitcomet_tooltip(theme, "Last delta (Ctrl+End)".into()),
                )
                .child(
                    components::Button::new("conflict_prev_unresolved", "")
                        .start_slot(svg_icon(
                            "icons/arrow_up.svg",
                            theme.colors.warning,
                            px(14.0),
                        ))
                        .style(components::ButtonStyle::Outlined)
                        .borderless()
                        .disabled(!can_prev_unresolved)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.conflict_jump_prev_unresolved(cx);
                        })
                        .gitcomet_tooltip(theme, "Previous unresolved conflict (Ctrl+PgUp)".into()),
                )
                .child(
                    components::Button::new("conflict_next_unresolved", "")
                        .start_slot(svg_icon(
                            "icons/arrow_down.svg",
                            theme.colors.warning,
                            px(14.0),
                        ))
                        .style(components::ButtonStyle::Outlined)
                        .borderless()
                        .disabled(!can_next_unresolved)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.conflict_jump_next_unresolved(cx);
                        })
                        .gitcomet_tooltip(theme, "Next unresolved conflict (Ctrl+PgDn)".into()),
                )
            })
            .when(
                !conflict_rendered_preview_active
                    && self.conflict_resolver.active_conflict.is_some(),
                |d| {
                    // section 30: visible pick affordances for the active conflict,
                    // mirroring the A/B/C/D quick-pick keys.
                    let active_ix = self
                        .conflict_resolver
                        .active_conflict
                        .expect("pick controls require an active displayed conflict");
                    let has_base = self
                        .conflict_resolver
                        .conflict_has_base
                        .get(active_ix)
                        .copied()
                        .unwrap_or(false);
                    let selected =
                        self.conflict_resolver_selected_choices_for_conflict_ix(active_ix);
                    let is_three_way =
                        self.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay;
                    let output_actions_enabled = !self.conflict_resolver.output_is_protected;
                    let mut pick_btn =
                        |id: &'static str,
                         label: &'static str,
                         hint: &'static str,
                         choice: conflict_resolver::ConflictChoice,
                         enabled: bool,
                         tooltip: &'static str| {
                            components::Button::new(id, label)
                                .style(components::ButtonStyle::Outlined)
                                .separated_end_slot(Self::diff_nav_hotkey_hint(theme, hint))
                                .selected(selected.contains(&choice))
                                .disabled(!enabled)
                                .on_click(theme, cx, move |this, _e, _w, cx| {
                                    this.conflict_resolver_pick_active_conflict(choice, cx);
                                })
                                .gitcomet_tooltip(theme, tooltip.into())
                        };
                    let cluster = d.child(div().w(px(1.0)).h(px(12.0)).bg(theme.colors.border));
                    if is_three_way {
                        cluster
                            .child(pick_btn(
                                "conflict_pick_base",
                                "Base",
                                "A",
                                conflict_resolver::ConflictChoice::Base,
                                has_base && output_actions_enabled,
                                "Pick the base (ancestor) version for the active conflict \
                                 (A or Ctrl+1; U un-resolves)",
                            ))
                            .child(pick_btn(
                                "conflict_pick_ours",
                                "Ours",
                                "B",
                                conflict_resolver::ConflictChoice::Ours,
                                output_actions_enabled,
                                "Pick the local (ours) version for the active conflict \
                                 (B or Ctrl+2; U un-resolves)",
                            ))
                            .child(pick_btn(
                                "conflict_pick_theirs",
                                "Theirs",
                                "C",
                                conflict_resolver::ConflictChoice::Theirs,
                                output_actions_enabled,
                                "Pick the incoming (theirs) version for the active conflict \
                                 (C or Ctrl+3; U un-resolves)",
                            ))
                            .child(pick_btn(
                                "conflict_pick_both",
                                "Both",
                                "D",
                                conflict_resolver::ConflictChoice::Both,
                                output_actions_enabled,
                                "Keep both versions (ours, then theirs) for the active conflict \
                                 (D; U un-resolves)",
                            ))
                    } else {
                        cluster
                            .child(pick_btn(
                                "conflict_pick_ours",
                                "Local",
                                "A",
                                conflict_resolver::ConflictChoice::Ours,
                                output_actions_enabled,
                                "Pick the local (ours) version for the active conflict \
                                 (A or Ctrl+1; U un-resolves)",
                            ))
                            .child(pick_btn(
                                "conflict_pick_theirs",
                                "Remote",
                                "B",
                                conflict_resolver::ConflictChoice::Theirs,
                                output_actions_enabled,
                                "Pick the incoming (theirs) version for the active conflict \
                                 (B or Ctrl+2; U un-resolves)",
                            ))
                            .child(pick_btn(
                                "conflict_pick_both",
                                "Both",
                                "C",
                                conflict_resolver::ConflictChoice::Both,
                                output_actions_enabled,
                                "Keep both versions (ours, then theirs) for the active conflict \
                                 (C or Ctrl+3; U un-resolves)",
                            ))
                    }
                },
            )
            .when_some(next_file_btn, |d, btn| d.child(btn));

        if let (Some(repo_id), Some(path)) = (repo_id, conflict_target_path.clone()) {
            let total = self.conflict_resolver_conflict_count();
            let resolved = self.conflict_resolver_resolved_count();
            let unresolved = total.saturating_sub(resolved);
            let focused_mergetool_mode = self.view_mode == GitCometViewMode::FocusedMergetool;
            let save_label = if focused_mergetool_mode {
                "Save & close"
            } else {
                "Save"
            };
            let save_path = path.clone();
            let stage_path = path.clone();
            let gate_unresolved = if self.conflict_resolver.output_is_protected {
                self.conflict_resolver_input.read_with(cx, |input, _| {
                    usize::from(conflict_resolver::text_contains_conflict_markers(
                        input.text(),
                    ))
                })
            } else if self.conflict_resolved_output_is_streamed() {
                unresolved
            } else {
                self.conflict_resolver_input.read_with(cx, |input, _| {
                    conflict_resolver::conflict_stage_safety_check(
                        input.text(),
                        &self.conflict_resolver.marker_segments,
                        &self.conflict_resolved_output_block_map,
                    )
                    .unresolved_blocks
                })
            };
            let save_button = components::Button::new("conflict_save", save_label)
                .style(components::ButtonStyle::Outlined)
                .disabled(gate_unresolved > 0)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    let text = this.current_conflict_resolved_output_text(cx);
                    let blocks_save = if this.conflict_resolver.output_is_protected {
                        conflict_resolver::text_contains_conflict_markers(&text)
                    } else {
                        conflict_resolver::conflict_stage_safety_check(
                            &text,
                            &this.conflict_resolver.marker_segments,
                            &this.conflict_resolved_output_block_map,
                        )
                        .blocks_save()
                    };
                    if blocks_save {
                        cx.notify();
                        return;
                    }
                    if this.view_mode == GitCometViewMode::FocusedMergetool {
                        this.focused_mergetool_save_and_exit(repo_id, save_path.clone(), cx);
                        return;
                    }
                    let text = this.conflict_resolver_save_contents_from_text(text);
                    this.store.dispatch(Msg::SaveWorktreeFile {
                        repo_id,
                        path: save_path.clone(),
                        contents: text,
                        stage: false,
                    });
                });
            controls = controls
                .child(div().w(px(1.0)).h(px(12.0)).bg(theme.colors.border))
                .child(save_button)
                .when(show_conflict_save_stage_action(self.view_mode), |d| {
                    let mut save_stage_btn =
                        components::Button::new("conflict_save_stage", "Save & stage")
                            .style(components::ButtonStyle::Filled)
                            .disabled(gate_unresolved > 0)
                            .on_click(theme, cx, move |this, _e, _window, cx| {
                                let text = this.current_conflict_resolved_output_text(cx);
                                let blocks_stage = if this.conflict_resolver.output_is_protected {
                                    conflict_resolver::text_contains_conflict_markers(&text)
                                } else {
                                    conflict_resolver::conflict_stage_safety_check(
                                        &text,
                                        &this.conflict_resolver.marker_segments,
                                        &this.conflict_resolved_output_block_map,
                                    )
                                    .blocks_save()
                                };
                                if blocks_stage {
                                    cx.notify();
                                } else {
                                    let text = this.conflict_resolver_save_contents_from_text(text);
                                    this.store.dispatch(Msg::SaveWorktreeFile {
                                        repo_id,
                                        path: stage_path.clone(),
                                        contents: text,
                                        stage: true,
                                    });
                                }
                            });
                    if gate_unresolved > 0 {
                        let noun = if gate_unresolved == 1 {
                            "conflict is"
                        } else {
                            "conflicts are"
                        };
                        save_stage_btn = save_stage_btn.gitcomet_tooltip(
                            theme,
                            format!("Disabled: {gate_unresolved} {noun} still unresolved").into(),
                        );
                    }
                    d.child(save_stage_btn)
                });
        }

        controls
    }

    /// Footer bar for the text conflict resolver (section 30): live resolution
    /// status line plus the leftover-marker indicator. The save/stage
    /// completion actions live in the top toolbar; merge abort lives in
    /// the action bar's "Abort merge" button.
    fn conflict_resolver_footer(
        &self,
        theme: AppTheme,
        can_reset_from_markers: bool,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let total = self.conflict_resolver_conflict_count();
        let resolved = self.conflict_resolver_resolved_count();
        let unresolved = total.saturating_sub(resolved);
        let auto_solved = self.conflict_resolver_auto_resolved_count().min(total);

        // The footer only needs the marker-presence bit. Scan the editor text
        // in place (`text()` borrows a `&str`) instead of cloning the whole
        // resolved output to a String on every render. Streamed mode never
        // materializes the text in the TextInput, so it has no markers to scan.
        let has_conflict_markers = !self.conflict_resolved_output_is_streamed()
            && self.conflict_resolver_input.read_with(cx, |i, _| {
                conflict_resolver::text_contains_conflict_markers(i.text())
            });

        let progress_label = (total > 0).then(|| {
            let mut label = format!("{resolved}/{total} resolved");
            if auto_solved > 0 {
                label.push_str(&format!(" · {auto_solved} auto-solved"));
            }
            SharedString::from(label)
        });

        let status: AnyElement = if total == 0 {
            div()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("No conflicts in this file")
                .into_any_element()
        } else if unresolved > 0 {
            let noun = if unresolved == 1 {
                "conflict"
            } else {
                "conflicts"
            };
            div()
                .id("conflict_resolver_status")
                .text_xs()
                .text_color(theme.colors.warning)
                .child(format!("⚠ {unresolved} {noun} unresolved"))
                .gitcomet_tooltip(
                    theme,
                    progress_label
                        .clone()
                        .unwrap_or_else(|| "Resolution progress".into()),
                )
                .into_any_element()
        } else {
            div()
                .id("conflict_resolver_status")
                .text_xs()
                .text_color(theme.colors.success)
                .child("✓ All conflicts resolved")
                .gitcomet_tooltip(
                    theme,
                    progress_label
                        .clone()
                        .unwrap_or_else(|| "Resolution complete".into()),
                )
                .into_any_element()
        };

        div()
            .id("conflict_resolver_footer")
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl_1()
                    .child(status)
                    .when(has_conflict_markers, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme.colors.danger)
                                .child("markers remain"),
                        )
                    }),
            )
            .child(
                components::Button::new("conflict_reset_markers", "Reset from markers")
                    .style(components::ButtonStyle::Transparent)
                    .disabled(!can_reset_from_markers)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.conflict_resolver_reset_output_from_markers(cx);
                    }),
            )
    }

    /// The main conflict resolver pane body: three-way / two-way source
    /// columns plus the merged output section.
    /// Segmented control for the overview column's comparison mode, matching
    /// kdiff3's Normal / A-B / A-C / B-C overview modes.
    fn conflict_overview_mode_selector(
        &self,
        theme: AppTheme,
        ui_scale_percent: u32,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        use gitcomet_core::merge::OverviewMode;

        let active = self.conflict_resolver.overview_mode;
        let selected_bg = theme.colors.active;
        let mut row = div()
            .id("conflict_overview_mode")
            .flex()
            .items_center()
            .h(components::control_height(ui_scale_percent))
            .rounded(px(theme.radii.row))
            .border_1()
            .border_color(theme.colors.border)
            .bg(gpui::rgba(0x00000000))
            .overflow_hidden()
            .p(px(1.0));

        for (ix, mode) in OverviewMode::ALL.into_iter().enumerate() {
            if ix > 0 {
                row = row.child(div().h_full().w(px(1.0)).bg(theme.colors.border));
            }
            let id = match mode {
                OverviewMode::Merge => "conflict_overview_merge",
                OverviewMode::BaseVsLocal => "conflict_overview_ab",
                OverviewMode::BaseVsRemote => "conflict_overview_ac",
                OverviewMode::LocalVsRemote => "conflict_overview_bc",
            };
            let tooltip = match mode {
                OverviewMode::Merge => {
                    "Overview: the merge — each side's changes and the conflicts"
                }
                OverviewMode::BaseVsLocal => "Overview: every line where Local differs from Base",
                OverviewMode::BaseVsRemote => "Overview: every line where Remote differs from Base",
                OverviewMode::LocalVsRemote => {
                    "Overview: every line where Local differs from Remote"
                }
            };
            row = row.child(
                components::Button::new(id, mode.label())
                    .borderless()
                    .style(components::ButtonStyle::Subtle)
                    .selected(active == mode)
                    .selected_bg(selected_bg)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        this.conflict_resolver.overview_mode = mode;
                        this.conflict_resolver.rebuild_overview_bands();
                        cx.notify();
                    })
                    .gitcomet_tooltip(theme, tooltip.into()),
            );
        }

        row.into_any_element()
    }

    pub(super) fn render_conflict_resolver_pane(
        &mut self,
        conflict_target_path: Option<std::path::PathBuf>,
        repo_id: Option<RepoId>,
        theme: AppTheme,
        ui_scale_percent: u32,
        editor_font_family: String,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let repo = self.active_repo();
        match (repo, conflict_target_path) {
            (None, _) => {
                components::empty_state(theme, "Resolve", "No repository.").into_any_element()
            }
            (_, None) => components::empty_state(theme, "Resolve", "No conflicted file selected.")
                .into_any_element(),
            (Some(repo), Some(path)) => {
                let title: SharedString =
                    format!("Resolve conflict: {}", self.cached_path_display(&path)).into();
                if let Some(repo_id) = repo_id {
                    match renderable_conflict_file(repo, &self.conflict_resolver, &path) {
                            RenderableConflictFile::Loading => {
                                components::empty_state(theme, title, "Loading conflict data…")
                                    .into_any_element()
                            }
                            RenderableConflictFile::Error(error) => {
                                components::empty_state(theme, title, error).into_any_element()
                            }
                            RenderableConflictFile::Missing => {
                                components::empty_state(theme, title, "No conflict data.")
                                    .into_any_element()
                            }
                            RenderableConflictFile::File(file)
                                if self.conflict_resolver.is_binary_conflict
                                    || conflict_file_is_binary(&file) =>
                            {
                                // Binary/non-UTF8 side-pick resolver panel.
                                self.render_binary_conflict_resolver(
                                    theme,
                                    repo_id,
                                    path,
                                    &file,
                                    cx,
                                )
                            }
                            RenderableConflictFile::File(file)
                                if matches!(
                                    self.conflict_resolver.strategy,
                                    Some(gitcomet_core::conflict_session::ConflictResolverStrategy::TwoWayKeepDelete)
                                ) =>
                            {
                                // Keep/delete resolver for modify/delete conflicts.
                                let kind = self.conflict_resolver.conflict_kind.unwrap_or(
                                    gitcomet_core::domain::FileConflictKind::DeletedByUs,
                                );
                                self.render_keep_delete_conflict_resolver(
                                    theme, repo_id, path, &file, kind, cx,
                                )
                            }
                            RenderableConflictFile::File(file)
                                if matches!(
                                    self.conflict_resolver.strategy,
                                    Some(gitcomet_core::conflict_session::ConflictResolverStrategy::DecisionOnly)
                                ) =>
                            {
                                // Decision-only resolver for BothDeleted conflicts.
                                self.render_decision_conflict_resolver(theme, repo_id, path, &file, cx)
                            }
                            RenderableConflictFile::File(file) => {
                            let has_current = file.current.is_some();

                            let view_mode = self.conflict_resolver.view_mode;

                            let diff_len = match view_mode {
                                ConflictResolverViewMode::ThreeWay => {
                                    self.conflict_resolver.three_way_visible_len()
                                }
                                ConflictResolverViewMode::TwoWayDiff => {
                                    self.conflict_resolver.two_way_visible_len()
                                }
                            };
                            // Keep intentional reading space below the source
                            // diffs. Resolved output has its own shorter range;
                            // scroll synchronization preserves a clamped
                            // follower without re-electing it as the master.
                            let diff_list_len = if diff_len > 0 {
                                diff_len + CONFLICT_BOTTOM_OVERSCROLL_ROWS
                            } else {
                                0
                            };

                            let conflict_count = self.conflict_resolver_conflict_count();
                            let active_conflict = self.conflict_resolver.active_conflict;
                            let has_conflicts = conflict_count > 0;
                            let resolved_count = self.conflict_resolver_resolved_count();
                            let active_autosolve_trace = repo
                                .conflict_state.conflict_session
                                .as_ref()
                                .and_then(|session| {
                                    let active_conflict = active_conflict?;
                                    conflict_resolver::active_conflict_autosolve_trace_label(
                                        session,
                                        &self.conflict_resolver.conflict_region_indices,
                                        active_conflict,
                                    )
                                })
                                .map(SharedString::from);

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
                                    let mut d = d;
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
                                })
                                ;

                            let preview_kind = super::super::preview_path_rendered_kind(&path);
                            let show_preview_toggle = preview_kind.is_some();
                            let preview_mode = self.conflict_resolver.resolver_preview_mode;
                            let is_rendered_preview_active =
                                show_preview_toggle
                                    && preview_mode == ConflictResolverPreviewMode::Preview;

                            // kdiff3's overview column sits left of the inputs
                            // and takes its width out of their budget.
                            let overview_w = if !is_rendered_preview_active
                                && self.conflict_resolver.has_overview()
                            {
                                px(components::OVERVIEW_COLUMN_WIDTH_PX)
                            } else {
                                px(0.0)
                            };
                            let overview_mode_selector = (overview_w > px(0.0)
                                && !self.conflict_resolver.three_way_text.base.is_empty())
                            .then(|| {
                                self.conflict_overview_mode_selector(theme, ui_scale_percent, cx)
                            });

                            let preview_toggle = show_preview_toggle.then(|| {
                                let view_toggle_border = theme.colors.border;
                                let view_toggle_selected_bg = theme.colors.active;
                                let view_toggle_divider = theme.colors.border;
                                div()
                                    .id("conflict_preview_toggle")
                                    .flex()
                                    .items_center()
                                    .h(components::control_height(ui_scale_percent))
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
                                            let _ = this.request_conflict_file_load_mode(
                                                gitcomet_state::model::ConflictFileLoadMode::Full,
                                            );
                                            cx.notify();
                                        }),
                                    )
                            });

                            let top_header = (preview_toggle.is_some()
                                || overview_mode_selector.is_some())
                            .then(|| {
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .when_some(overview_mode_selector, |d, selector| {
                                                d.child(selector)
                                            }),
                                    )
                                    .when_some(preview_toggle, |d, toggle| d.child(toggle))
                            });

                            // Compute three-way column widths
                            let vertical_sync_enabled =
                                self.diff_scroll_sync.includes_vertical();
                            let scrollbar_gutter = if vertical_sync_enabled {
                                components::Scrollbar::visible_gutter(
                                    self.conflict_resolver_diff_scroll.clone(),
                                    components::ScrollbarAxis::Vertical,
                                )
                            } else {
                                px(0.0)
                            };
                            let handle_w = px(PANE_RESIZE_HANDLE_PX);
                            let min_col_w = px(DIFF_SPLIT_COL_MIN_PX);
                            let main_w = (self.main_pane_content_width(cx)
                                - scrollbar_gutter
                                - overview_w)
                                .max(px(0.0));
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

                            let active_hsplit_resize = self.conflict_hsplit_resize;
                            let conflict_hsplit_resize_handle =
                                |id: &'static str, which: ConflictHSplitResizeHandle| {
                                    let dragging = active_hsplit_resize
                                        .is_some_and(|state| state.handle == which);
                                    div()
                                        .id(id)
                                        .group(id)
                                        .w(handle_w)
                                        .h_full()
                                        .cursor(CursorStyle::ResizeLeftRight)
                                        .child(components::resize_grip(
                                            theme,
                                            ui_scale_percent,
                                            id,
                                            components::ResizeGripAxis::Vertical,
                                            dragging,
                                            Some(theme.colors.border),
                                        ))
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

                                                let scrollbar_gutter = if this
                                                    .diff_scroll_sync
                                                    .includes_vertical()
                                                {
                                                    components::Scrollbar::visible_gutter(
                                                        this.conflict_resolver_diff_scroll.clone(),
                                                        components::ScrollbarAxis::Vertical,
                                                    )
                                                } else {
                                                    px(0.0)
                                                };
                                                let main_w =
                                                    (this.main_pane_content_width(cx)
                                                        - scrollbar_gutter)
                                                        .max(px(0.0));
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

                            let conflict_diff_split_dragging =
                                self.conflict_diff_split_resize.is_some();
                            let conflict_diff_split_resize_handle = |id: &'static str| {
                                div()
                                    .id(id)
                                    .group(id)
                                    .w(handle_w)
                                    .h_full()
                                    .cursor(CursorStyle::ResizeLeftRight)
                                    .child(components::resize_grip(
                                        theme,
                                        ui_scale_percent,
                                        id,
                                        components::ResizeGripAxis::Vertical,
                                        conflict_diff_split_dragging,
                                        Some(theme.colors.border),
                                    ))
                                    .on_drag(
                                        ConflictDiffSplitResizeHandle::Divider,
                                        |_, _, _, cx| {
                                            cx.new(|_| ConflictDiffSplitResizeDragGhost)
                                        },
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, e: &MouseDownEvent, _w, cx| {
                                            cx.stop_propagation();
                                            this.conflict_diff_split_resize =
                                                Some(ConflictDiffSplitResizeState {
                                                    start_x: e.position.x,
                                                    start_ratio: this.conflict_diff_split_ratio,
                                                });
                                            cx.notify();
                                        }),
                                    )
                                    .on_drag_move(cx.listener(
                                        |this,
                                         e: &gpui::DragMoveEvent<
                                            ConflictDiffSplitResizeHandle,
                                        >,
                                         _w,
                                         cx| {
                                            let Some(state) = this.conflict_diff_split_resize else {
                                                return;
                                            };
                                            let Some(new_ratio) = next_conflict_diff_split_ratio(
                                                state,
                                                e.event.position.x,
                                                this.conflict_diff_split_col_widths,
                                            ) else {
                                                return;
                                            };
                                            if (this.conflict_diff_split_ratio - new_ratio).abs()
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
                                    )
                            };

                            let top_title_row = div()
                                .h(px(22.0))
                                .w_full()
                                .flex()
                                .items_center()
                                // Keep the column headers over their columns:
                                // the overview column shifts the body right.
                                .when(overview_w > px(0.0), |d| {
                                    d.child(div().w(overview_w).h_full().flex_shrink_0())
                                })
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
                                    .child(conflict_diff_split_resize_handle(
                                        "conflict_diff_split_header_handle",
                                    ))
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
                                });

                            let top_body: AnyElement = if diff_len == 0 {
                                components::empty_state(theme, "Inputs", "Stage data not available.")
                                    .into_any_element()
                            } else if is_rendered_preview_active {
                                match preview_kind {
                                    Some(RenderedPreviewKind::Svg) => self
                                        .render_conflict_resolver_svg_preview(theme, cx),
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
                                // Sync vertical scrolling across per-column lists.
                                self.sync_conflict_preview_scroll();

                                match view_mode {
                                    ConflictResolverViewMode::ThreeWay => {
                                        let base_scrollbar_gutter =
                                            components::Scrollbar::visible_gutter(
                                                self.conflict_resolver_diff_scroll.clone(),
                                                components::ScrollbarAxis::Vertical,
                                            );
                                        let ours_scrollbar_gutter =
                                            components::Scrollbar::visible_gutter(
                                                self.conflict_preview_ours_scroll.clone(),
                                                components::ScrollbarAxis::Vertical,
                                            );
                                        let theirs_scrollbar_gutter =
                                            components::Scrollbar::visible_gutter(
                                                self.conflict_preview_theirs_scroll.clone(),
                                                components::ScrollbarAxis::Vertical,
                                            );
                                        let base_list = uniform_list(
                                            "conflict_three_way_base_list",
                                            diff_list_len,
                                            cx.processor(Self::render_conflict_three_way_base_rows),
                                        )
                                        .with_width_from_item(Some(
                                            self.conflict_resolver
                                                .three_way_horizontal_measure_row(
                                                    ThreeWayColumn::Base,
                                                ),
                                        ))
                                        .h_full()
                                        .min_h(px(0.0))
                                        .with_horizontal_sizing_behavior(
                                            gpui::ListHorizontalSizingBehavior::Unconstrained,
                                        )
                                        .track_scroll(&self.conflict_resolver_diff_scroll)
                                        .on_scroll_wheel(cx.listener(
                                            |this, e: &gpui::ScrollWheelEvent, window, cx| {
                                                if e.delta.pixel_delta(window.line_height()).y
                                                    != px(0.0)
                                                {
                                                    this.record_conflict_vertical_wheel_master(0);
                                                    cx.notify();
                                                }
                                            },
                                        ));

                                        let ours_list = uniform_list(
                                            "conflict_three_way_ours_list",
                                            diff_list_len,
                                            cx.processor(Self::render_conflict_three_way_ours_rows),
                                        )
                                        .with_width_from_item(Some(
                                            self.conflict_resolver
                                                .three_way_horizontal_measure_row(
                                                    ThreeWayColumn::Ours,
                                                ),
                                        ))
                                        .h_full()
                                        .min_h(px(0.0))
                                        .with_horizontal_sizing_behavior(
                                            gpui::ListHorizontalSizingBehavior::Unconstrained,
                                        )
                                        .track_scroll(&self.conflict_preview_ours_scroll)
                                        .on_scroll_wheel(cx.listener(
                                            |this, e: &gpui::ScrollWheelEvent, window, cx| {
                                                if e.delta.pixel_delta(window.line_height()).y
                                                    != px(0.0)
                                                {
                                                    this.record_conflict_vertical_wheel_master(1);
                                                    cx.notify();
                                                }
                                            },
                                        ));

                                        let theirs_list = uniform_list(
                                            "conflict_three_way_theirs_list",
                                            diff_list_len,
                                            cx.processor(Self::render_conflict_three_way_theirs_rows),
                                        )
                                        .with_width_from_item(Some(
                                            self.conflict_resolver
                                                .three_way_horizontal_measure_row(
                                                    ThreeWayColumn::Theirs,
                                                ),
                                        ))
                                        .h_full()
                                        .min_h(px(0.0))
                                        .with_horizontal_sizing_behavior(
                                            gpui::ListHorizontalSizingBehavior::Unconstrained,
                                        )
                                        .track_scroll(&self.conflict_preview_theirs_scroll)
                                        .on_scroll_wheel(cx.listener(
                                            |this, e: &gpui::ScrollWheelEvent, window, cx| {
                                                if e.delta.pixel_delta(window.line_height()).y
                                                    != px(0.0)
                                                {
                                                    this.record_conflict_vertical_wheel_master(2);
                                                    cx.notify();
                                                }
                                            },
                                        ));

                                        let shared_scrollbar_gutter =
                                            if vertical_sync_enabled {
                                                base_scrollbar_gutter
                                            } else {
                                                px(0.0)
                                            };
                                        div()
                                            .id("conflict_resolver_diff_scroll")
                                            .relative()
                                            .flex_1()
                                            .min_h(px(0.0))
                                            .bg(theme.colors.window_bg)
                                            .font_family(editor_font_family.clone())
                                            .flex()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .h_full()
                                                    .min_h(px(0.0))
                                                    .flex()
                                                    .pr(shared_scrollbar_gutter)
                                                    .child(
                                                        div()
                                                            .relative()
                                                            .w(col_a_w)
                                                            .min_w(px(0.0))
                                                            .h_full()
                                                            .child(
                                                                div()
                                                                    .h_full()
                                                                    .min_h(px(0.0))
                                                                    .pr(
                                                                        if vertical_sync_enabled {
                                                                            px(0.0)
                                                                        } else {
                                                                            base_scrollbar_gutter
                                                                        },
                                                                    )
                                                                    .child(base_list),
                                                            )
                                                            .when(!vertical_sync_enabled, |d| {
                                                                d.child(
                                                                    components::Scrollbar::new(
                                                                        "conflict_base_scrollbar",
                                                                        self.conflict_resolver_diff_scroll.clone(),
                                                                    )
                                                                    .always_visible()
                                                                    .render(theme),
                                                                )
                                                            })
                                                            .child(
                                                                components::Scrollbar::horizontal(
                                                                    "conflict_base_hscrollbar",
                                                                    self.conflict_resolver_diff_scroll.clone(),
                                                                )
                                                                .always_visible()
                                                                .render(theme),
                                                            ),
                                                    )
                                                    .child(conflict_hsplit_resize_handle(
                                                        "conflict_hsplit_body_first",
                                                        ConflictHSplitResizeHandle::First,
                                                    ))
                                                    .child(
                                                        div()
                                                            .relative()
                                                            .w(col_b_w)
                                                            .min_w(px(0.0))
                                                            .h_full()
                                                            .child(
                                                                div()
                                                                    .h_full()
                                                                    .min_h(px(0.0))
                                                                    .pr(
                                                                        if vertical_sync_enabled {
                                                                            px(0.0)
                                                                        } else {
                                                                            ours_scrollbar_gutter
                                                                        },
                                                                    )
                                                                    .child(ours_list),
                                                            )
                                                            .when(!vertical_sync_enabled, |d| {
                                                                d.child(
                                                                    components::Scrollbar::new(
                                                                        "conflict_ours_scrollbar",
                                                                        self.conflict_preview_ours_scroll.clone(),
                                                                    )
                                                                    .always_visible()
                                                                    .render(theme),
                                                                )
                                                            })
                                                            .child(
                                                                components::Scrollbar::horizontal(
                                                                    "conflict_ours_hscrollbar",
                                                                    self.conflict_preview_ours_scroll.clone(),
                                                                )
                                                                .always_visible()
                                                                .render(theme),
                                                            ),
                                                    )
                                                    .child(conflict_hsplit_resize_handle(
                                                        "conflict_hsplit_body_second",
                                                        ConflictHSplitResizeHandle::Second,
                                                    ))
                                                    .child(
                                                        div()
                                                            .relative()
                                                            .w(col_c_w)
                                                            .flex_grow()
                                                            .min_w(px(0.0))
                                                            .h_full()
                                                            .child(
                                                                div()
                                                                    .h_full()
                                                                    .min_h(px(0.0))
                                                                    .pr(
                                                                        if vertical_sync_enabled {
                                                                            px(0.0)
                                                                        } else {
                                                                            theirs_scrollbar_gutter
                                                                        },
                                                                    )
                                                                    .child(theirs_list),
                                                            )
                                                            .when(!vertical_sync_enabled, |d| {
                                                                d.child(
                                                                    components::Scrollbar::new(
                                                                        "conflict_theirs_scrollbar",
                                                                        self.conflict_preview_theirs_scroll.clone(),
                                                                    )
                                                                    .always_visible()
                                                                    .render(theme),
                                                                )
                                                            })
                                                            .child(
                                                                components::Scrollbar::horizontal(
                                                                    "conflict_theirs_hscrollbar",
                                                                    self.conflict_preview_theirs_scroll.clone(),
                                                                )
                                                                .always_visible()
                                                                .render(theme),
                                                            ),
                                                    ),
                                            )
                                            .when(vertical_sync_enabled, |d| {
                                                d.child(
                                                    components::Scrollbar::new(
                                                        "conflict_resolver_diff_scrollbar",
                                                        self.conflict_resolver_diff_scroll.clone(),
                                                    )
                                                    .always_visible()
                                                    .render(theme),
                                                )
                                            })
                                            .into_any_element()
                                    }
                                    ConflictResolverViewMode::TwoWayDiff => {
                                        let [left_w, right_w] =
                                            self.conflict_diff_split_col_widths;
                                        let left_scrollbar_gutter =
                                            components::Scrollbar::visible_gutter(
                                                self.conflict_resolver_diff_scroll.clone(),
                                                components::ScrollbarAxis::Vertical,
                                            );
                                        let right_scrollbar_gutter =
                                            components::Scrollbar::visible_gutter(
                                                self.conflict_preview_theirs_scroll.clone(),
                                                components::ScrollbarAxis::Vertical,
                                            );

                                        let left_list = uniform_list(
                                            "conflict_diff_left_list",
                                            diff_list_len,
                                            cx.processor(Self::render_conflict_diff_left_rows),
                                        )
                                        .with_width_from_item(Some(
                                            self.conflict_resolver
                                                .two_way_horizontal_measure_row(
                                                    conflict_resolver::ConflictPickSide::Ours,
                                                ),
                                        ))
                                        .h_full()
                                        .min_h(px(0.0))
                                        .with_horizontal_sizing_behavior(
                                            gpui::ListHorizontalSizingBehavior::Unconstrained,
                                        )
                                        .track_scroll(&self.conflict_resolver_diff_scroll)
                                        .on_scroll_wheel(cx.listener(
                                            |this, e: &gpui::ScrollWheelEvent, window, cx| {
                                                if e.delta.pixel_delta(window.line_height()).y
                                                    != px(0.0)
                                                {
                                                    this.record_conflict_vertical_wheel_master(0);
                                                    cx.notify();
                                                }
                                            },
                                        ));

                                        let right_list = uniform_list(
                                            "conflict_diff_right_list",
                                            diff_list_len,
                                            cx.processor(Self::render_conflict_diff_right_rows),
                                        )
                                        .with_width_from_item(Some(
                                            self.conflict_resolver
                                                .two_way_horizontal_measure_row(
                                                    conflict_resolver::ConflictPickSide::Theirs,
                                                ),
                                        ))
                                        .h_full()
                                        .min_h(px(0.0))
                                        .with_horizontal_sizing_behavior(
                                            gpui::ListHorizontalSizingBehavior::Unconstrained,
                                        )
                                        .track_scroll(&self.conflict_preview_theirs_scroll)
                                        .on_scroll_wheel(cx.listener(
                                            |this, e: &gpui::ScrollWheelEvent, window, cx| {
                                                if e.delta.pixel_delta(window.line_height()).y
                                                    != px(0.0)
                                                {
                                                    this.record_conflict_vertical_wheel_master(2);
                                                    cx.notify();
                                                }
                                            },
                                        ));

                                        let shared_scrollbar_gutter =
                                            if vertical_sync_enabled {
                                                left_scrollbar_gutter
                                            } else {
                                                px(0.0)
                                            };
                                        div()
                                            .id("conflict_resolver_diff_scroll")
                                            .relative()
                                            .flex_1()
                                            .min_h(px(0.0))
                                            .bg(theme.colors.window_bg)
                                            .font_family(editor_font_family.clone())
                                            .flex()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .h_full()
                                                    .min_h(px(0.0))
                                                    .flex()
                                                    .pr(shared_scrollbar_gutter)
                                                    .child(
                                                        div()
                                                            .relative()
                                                            .w(left_w)
                                                            .min_w(px(0.0))
                                                            .h_full()
                                                            .child(
                                                                div()
                                                                    .h_full()
                                                                    .min_h(px(0.0))
                                                                    .pr(
                                                                        if vertical_sync_enabled {
                                                                            px(0.0)
                                                                        } else {
                                                                            left_scrollbar_gutter
                                                                        },
                                                                    )
                                                                    .child(left_list),
                                                            )
                                                            .when(!vertical_sync_enabled, |d| {
                                                                d.child(
                                                                    components::Scrollbar::new(
                                                                        "conflict_diff_left_scrollbar",
                                                                        self.conflict_resolver_diff_scroll.clone(),
                                                                    )
                                                                    .always_visible()
                                                                    .render(theme),
                                                                )
                                                            })
                                                            .child(
                                                                components::Scrollbar::horizontal(
                                                                    "conflict_diff_left_hscrollbar",
                                                                    self.conflict_resolver_diff_scroll.clone(),
                                                                )
                                                                .always_visible()
                                                                .render(theme),
                                                            ),
                                                    )
                                                    .child(conflict_diff_split_resize_handle(
                                                        "conflict_diff_split_body_handle",
                                                    ))
                                                    .child(
                                                        div()
                                                            .relative()
                                                            .w(right_w)
                                                            .flex_grow()
                                                            .min_w(px(0.0))
                                                            .h_full()
                                                            .child(
                                                                div()
                                                                    .h_full()
                                                                    .min_h(px(0.0))
                                                                    .pr(
                                                                        if vertical_sync_enabled {
                                                                            px(0.0)
                                                                        } else {
                                                                            right_scrollbar_gutter
                                                                        },
                                                                    )
                                                                    .child(right_list),
                                                            )
                                                            .when(!vertical_sync_enabled, |d| {
                                                                d.child(
                                                                    components::Scrollbar::new(
                                                                        "conflict_diff_right_scrollbar",
                                                                        self.conflict_preview_theirs_scroll.clone(),
                                                                    )
                                                                    .always_visible()
                                                                    .render(theme),
                                                                )
                                                            })
                                                            .child(
                                                                components::Scrollbar::horizontal(
                                                                    "conflict_diff_right_hscrollbar",
                                                                    self.conflict_preview_theirs_scroll.clone(),
                                                                )
                                                                .always_visible()
                                                                .render(theme),
                                                            ),
                                                    ),
                                            )
                                            .when(vertical_sync_enabled, |d| {
                                                d.child(
                                                    components::Scrollbar::new(
                                                        "conflict_resolver_diff_scrollbar",
                                                        self.conflict_resolver_diff_scroll.clone(),
                                                    )
                                                    .always_visible()
                                                    .render(theme),
                                                )
                                            })
                                            .into_any_element()
                                    }
                                }
                            };

                            // kdiff3's Overview widget: the whole-file change
                            // map beside the inputs, framing the viewport and
                            // jumping the panes on click.
                            let top_body: AnyElement = if overview_w > px(0.0) {
                                let view = cx.entity();
                                let jump_rows = diff_list_len;
                                div()
                                    .flex()
                                    .flex_1()
                                    .h_full()
                                    .min_h(px(0.0))
                                    .child(
                                        components::OverviewColumn::new(
                                            "conflict_overview",
                                            self.conflict_resolver.overview_bands.clone(),
                                        )
                                        .compare_bands(
                                            self.conflict_resolver.overview_compare_bands.clone(),
                                        )
                                        .driver(self.conflict_resolver_diff_scroll.clone())
                                        .on_jump(move |fraction, _window, cx| {
                                            if jump_rows == 0 {
                                                return;
                                            }
                                            let row = ((fraction * jump_rows as f32) as usize)
                                                .min(jump_rows - 1);
                                            view.update(cx, |this, cx| {
                                                this.conflict_resolver_scroll_all_columns(
                                                    row,
                                                    gpui::ScrollStrategy::Center,
                                                );
                                                cx.notify();
                                            });
                                        })
                                        .render(theme),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .h_full()
                                            .min_h(px(0.0))
                                            .child(top_body),
                                    )
                                    .into_any_element()
                            } else {
                                top_body
                            };

                            let output_modified = self.conflict_resolved_output_is_modified();
                            let output_header = div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .px_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .text_xs()
                                        .text_color(theme.colors.text_muted)
                                        .child("Resolved output")
                                        .when(output_modified, |d| {
                                            d.child(
                                                div()
                                                    .id("conflict_resolved_output_modified")
                                                    .text_color(theme.colors.warning)
                                                    .child("[Modified]"),
                                            )
                                        }),
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
                                .group("conflict_resolver_vsplit_handle")
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom(px(-4.0))
                                .w_full()
                                .h(handle_h)
                                .cursor(CursorStyle::ResizeUpDown)
                                .child(components::resize_grip(
                                    theme,
                                    ui_scale_percent,
                                    "conflict_resolver_vsplit_handle",
                                    components::ResizeGripAxis::Horizontal,
                                    self.conflict_resolver_vsplit_resize.is_some(),
                                    Some(theme.colors.border),
                                ))
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
                                .py_2()
                                .gap_1()
                                .when_some(top_header, |d, header| d.child(header))
                                .child({
                                    let mut top_section = div()
                                        .relative()
                                        .min_h(min_section_h)
                                        .child(
                                            div()
                                                .absolute()
                                                .inset_0()
                                                .overflow_hidden()
                                                .flex()
                                                .flex_col()
                                                .child(top_title_row)
                                                .child(
                                                    div()
                                                        .border_t_1()
                                                        .border_color(theme.colors.border),
                                                )
                                                .child(top_body),
                                        )
                                        .child(vsplit_handle);
                                    top_section.style().flex_grow = Some(vsplit_ratio);
                                    top_section.style().flex_shrink = Some(1.0);
                                    top_section.style().flex_basis = Some(relative(0.).into());
                                    top_section
                                })
                                .child(output_header)
                                .when_some(autosolve_summary, |d, summary| {
                                    // section 30: make the autosolve pass visible on
                                    // open — accent chip, not a muted footnote.
                                    d.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .px_2()
                                            .py_0p5()
                                            .rounded(px(theme.radii.row))
                                            .bg(with_alpha(
                                                theme.colors.accent,
                                                if theme.is_dark { 0.14 } else { 0.10 },
                                            ))
                                            .text_xs()
                                            .text_color(theme.colors.accent)
                                            .child(summary),
                                    )
                                })
                                .child({
                                    self.prepare_conflict_resolved_output_editor(cx);
                                    self.ensure_resolved_output_visible_projection();
                                    self.sync_conflict_resolved_output_gutter_scroll();
                                    let mut bottom_section =
                                        div()
                                            .id("conflict_resolver_output")
                                            .min_h(min_section_h)
                                            .overflow_hidden()
                                            .flex()
                                            .flex_col()
                                            .bg(theme.colors.window_bg)
                                            .child(
                                                {
                                                    // Fold-projected row count in collapsed
                                                    // context mode; plain line count otherwise.
                                                    // Resolved output stops at its final real line;
                                                    // only source diffs get comfort overscroll. Keep
                                                    // the code and numbered gutter on exactly the
                                                    // same output range.
                                                    let outline_len =
                                                        self.resolved_output_visible_len();
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
                                                        &self.conflict_resolved_preview_gutter_scroll,
                                                    );

                                                    // Above the size guard the merged output stays
                                                    // read-only streamed (rendered from the
                                                    // projection); otherwise it is the editable
                                                    // free-text `TextInput`.
                                                    let streamed =
                                                        self.conflict_resolved_output_is_streamed();
                                                    let output_gutter_scroll =
                                                        self.conflict_resolved_preview_gutter_scroll
                                                            .clone();
                                                    let mirror_deferred_output_scroll =
                                                        output_gutter_scroll
                                                            .0
                                                            .borrow()
                                                            .deferred_scroll_to_item
                                                            .is_some();
                                                    let output_editor_scroll =
                                                        self.conflict_resolved_output_editor_scroll
                                                            .clone();
                                                    div()
                                                        // `scroll_to_item_strict` is consumed by the
                                                        // virtualized gutter during prepaint, after the
                                                        // normal gutter/editor synchronizer has already
                                                        // run for this frame. Mirror that newly-applied
                                                        // offset immediately and request the follow-up
                                                        // paint, so conflict navigation updates the
                                                        // editable output without waiting for incidental
                                                        // input (for example, mouse movement).
                                                        .when(
                                                            !streamed
                                                                && mirror_deferred_output_scroll,
                                                            move |d| {
                                                            d.on_children_prepainted(
                                                                move |_bounds, window, _cx| {
                                                                    let gutter_y =
                                                                        output_gutter_scroll
                                                                            .0
                                                                            .borrow()
                                                                            .base_handle
                                                                            .offset()
                                                                            .y;
                                                                    let editor_offset =
                                                                        output_editor_scroll.offset();
                                                                    let target_y =
                                                                        conflict_output_post_layout_scroll_y(
                                                                            gutter_y,
                                                                            output_editor_scroll
                                                                                .max_offset()
                                                                                .y,
                                                                        );
                                                                    if editor_offset.y != target_y {
                                                                        output_editor_scroll.set_offset(
                                                                            point(
                                                                                editor_offset.x,
                                                                                target_y,
                                                                            ),
                                                                        );
                                                                        window.refresh();
                                                                    }
                                                                },
                                                            )
                                                            },
                                                        )
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
                                                                .font_family(editor_font_family.clone())
                                                                // Forward horizontal wheel input to the narrower diff
                                                                // columns immediately; the normal bidirectional sync also
                                                                // keeps the output's content-width handle aligned.
                                                                .on_scroll_wheel(cx.listener(
                                                                    |this,
                                                                     e: &gpui::ScrollWheelEvent,
                                                                    window,
                                                                     cx| {
                                                                        let delta = e.delta.pixel_delta(window.line_height());
                                                                        if delta.y != px(0.0) {
                                                                            this.record_conflict_vertical_wheel_master(3);
                                                                        }
                                                                        let horizontal_changed = this
                                                                            .forward_conflict_output_horizontal_wheel(
                                                                                e, window,
                                                                            );
                                                                        // Vertical output scrolling is native to the editor,
                                                                        // but the separate virtualized line-number gutter is
                                                                        // synchronized from the parent render pass. Keep that
                                                                        // pass scheduled for real vertical gestures; otherwise
                                                                        // the gutter can update only when scrolling reaches a
                                                                        // boundary and another render happens to occur.
                                                                        if conflict_output_wheel_requires_notify(
                                                                            delta.y,
                                                                            horizontal_changed,
                                                                        ) {
                                                                            cx.notify();
                                                                        }
                                                                    },
                                                                ))
                                                                .when(
                                                                    !self
                                                                        .conflict_resolved_output_is_streamed(),
                                                                    |d| {
                                                                        d.on_mouse_down(
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
                                                                    },
                                                                )
                                                                .child(
                                                                    div()
                                                                        .flex()
                                                                        .items_start()
                                                                        .h_full()
                                                                        .min_h(px(0.0))
                                                                        // Bound (not min) the width so the editable output's
                                                                        // scroll container clips and overflows horizontally
                                                                        // instead of the whole row growing to the content width.
                                                                        .w_full()
                                                                        .min_w(px(0.0))
                                                                        .pr(
                                                                            if streamed {
                                                                                components::Scrollbar::visible_gutter(
                                                                                    self.conflict_resolved_preview_scroll.clone(),
                                                                                    components::ScrollbarAxis::Vertical,
                                                                                )
                                                                            } else {
                                                                                components::Scrollbar::visible_gutter(
                                                                                    self.conflict_resolved_output_editor_scroll.clone(),
                                                                                    components::ScrollbarAxis::Vertical,
                                                                                )
                                                                            },
                                                                        )
                                                                        // The gutter also carries the marker lane and
                                                                        // A/B/C origin badges (which conflict each output
                                                                        // line was picked for), so it stays even when line
                                                                        // numbers are hidden — only its width shrinks.
                                                                        .child(
                                                                            div()
                                                                                .id("conflict_resolver_output_gutter")
                                                                                // Hug the gutter content (marker + digit-sized
                                                                                // line-number cell + badge) so the marker stays
                                                                                // pinned far-left and the badge/divider sit right
                                                                                // against the code; the width tracks the file's
                                                                                // line-number digit count.
                                                                                .w(crate::view::rows::resolved_output_gutter_width(
                                                                                    self.conflict_resolved_preview_line_count,
                                                                                    self.mergetool_show_line_numbers,
                                                                                ))
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
                                                                                .child(if streamed {
                                                                                    // Read-only streamed output: a virtualized
                                                                                    // uniform_list over the projection, scrolling
                                                                                    // both axes so it stays coupled to the columns.
                                                                                    uniform_list(
                                                                                        "conflict_resolved_output_list",
                                                                                        outline_len,
                                                                                        cx.processor(
                                                                                            Self::render_conflict_resolved_output_rows,
                                                                                        ),
                                                                                    )
                                                                                    .with_width_from_item(Some(
                                                                                        self.conflict_resolved_output_measure_row,
                                                                                    ))
                                                                                    .h_full()
                                                                                    .min_h(px(0.0))
                                                                                    .track_scroll(
                                                                                        &self.conflict_resolved_preview_scroll,
                                                                                    )
                                                                                    .with_horizontal_sizing_behavior(
                                                                                        gpui::ListHorizontalSizingBehavior::Unconstrained,
                                                                                    )
                                                                                    .into_any_element()
                                                                                } else {
                                                                                    // The editable resolved-output pane: the
                                                                                    // `TextInput` lays out at full content size
                                                                                    // inside this `overflow_scroll` container, which
                                                                                    // owns the shared scroll handle the input reads
                                                                                    // to window its shaping. Scrolling both axes
                                                                                    // keeps it coupled to the columns horizontally.
                                                                                    div()
                                                                                        .id("conflict_resolver_output_editor_scroll")
                                                                                        .h_full()
                                                                                        .min_h(px(0.0))
                                                                                        // flex-col so the input stacks above the
                                                                                        // bottom overscroll spacer (vertical overflow)
                                                                                        // and, on the cross axis, the content-width
                                                                                        // input overflows to the right instead of
                                                                                        // being main-axis flex-shrunk to the viewport —
                                                                                        // which is what gives the container a
                                                                                        // horizontal `max_offset` to scroll-sync.
                                                                                        .flex()
                                                                                        .flex_col()
                                                                                        .items_start()
                                                                                        .w_full()
                                                                                        .min_w(px(0.0))
                                                                                        .overflow_scroll()
                                                                                        .track_scroll(
                                                                                            &self.conflict_resolved_output_editor_scroll,
                                                                                        )
                                                                                        .child(self.conflict_resolver_input.clone())
                                                                                        .into_any_element()
                                                                                }),
                                                                        ),
                                                                ),
                                                        )
                                                        .child(if streamed {
                                                            components::Scrollbar::new(
                                                                "conflict_resolver_output_scrollbar",
                                                                self.conflict_resolved_preview_scroll
                                                                    .clone(),
                                                            )
                                                            .always_visible()
                                                            .render(theme)
                                                            .into_any_element()
                                                        } else {
                                                            components::Scrollbar::new(
                                                                "conflict_resolver_output_scrollbar",
                                                                self.conflict_resolved_output_editor_scroll
                                                                    .clone(),
                                                            )
                                                            .always_visible()
                                                            .render(theme)
                                                            .into_any_element()
                                                        })
                                                        .child(if streamed {
                                                            components::Scrollbar::horizontal(
                                                                "conflict_resolver_output_hscrollbar",
                                                                self.conflict_resolved_preview_scroll
                                                                    .clone(),
                                                            )
                                                            .render(theme)
                                                            .into_any_element()
                                                        } else {
                                                            components::Scrollbar::horizontal(
                                                                "conflict_resolver_output_hscrollbar",
                                                                self.conflict_resolved_output_editor_scroll
                                                                    .clone(),
                                                            )
                                                            .render(theme)
                                                            .into_any_element()
                                                        })
                                                },
                                            );
                                    bottom_section.style().flex_grow = Some(1.0 - vsplit_ratio);
                                    bottom_section.style().flex_shrink = Some(1.0);
                                    bottom_section.style().flex_basis = Some(relative(0.).into());
                                    bottom_section
                                })
                                .child(self.conflict_resolver_footer(theme, has_current, cx))
                                // section 30 split: ends a row-drag on mouse-up anywhere.
                                .child(ConflictRowSelectionTracker { view: cx.entity() })
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
    }

    fn render_conflict_resolver_svg_preview(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        self.ensure_conflict_image_preview_cache(cx);

        let base_has_source = !self.conflict_resolver.three_way_text.base.is_empty();
        let ours_has_source = !self.conflict_resolver.three_way_text.ours.is_empty();
        let theirs_has_source = !self.conflict_resolver.three_way_text.theirs.is_empty();
        let base_img = self
            .conflict_resolver
            .image_preview
            .image(ThreeWayColumn::Base)
            .clone();
        let ours_img = self
            .conflict_resolver
            .image_preview
            .image(ThreeWayColumn::Ours)
            .clone();
        let theirs_img = self
            .conflict_resolver
            .image_preview
            .image(ThreeWayColumn::Theirs)
            .clone();

        let preview_cell = |id: &'static str,
                            label: &'static str,
                            image: Loadable<Option<Arc<gpui::Image>>>,
                            has_source: bool| {
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
                        .child(match image {
                            Loadable::Ready(Some(data)) => gpui::img(data)
                                .w_full()
                                .h_full()
                                .object_fit(gpui::ObjectFit::Contain)
                                .into_any_element(),
                            Loadable::NotLoaded | Loadable::Loading if has_source => div()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("Processing preview...")
                                .into_any_element(),
                            Loadable::Error(error) => div()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child(error)
                                .into_any_element(),
                            Loadable::Ready(None) if has_source => div()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("Preview unavailable.")
                                .into_any_element(),
                            _ => div()
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
            .child(preview_cell(
                "conflict_preview_base",
                "Base (A)",
                base_img,
                base_has_source,
            ))
            .child(preview_cell(
                "conflict_preview_ours",
                "Local (B)",
                ours_img,
                ours_has_source,
            ))
            .child(preview_cell(
                "conflict_preview_theirs",
                "Remote (C)",
                theirs_img,
                theirs_has_source,
            ))
            .into_any_element()
    }

    fn render_conflict_resolver_markdown_preview(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        self.ensure_conflict_markdown_preview_cache();
        self.sync_conflict_preview_scroll();

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
        let vertical_sync_enabled = self.diff_scroll_sync.includes_vertical();
        let scrollbar_gutter = if vertical_sync_enabled {
            components::Scrollbar::visible_gutter(
                vertical_handle.clone(),
                components::ScrollbarAxis::Vertical,
            )
        } else {
            px(0.0)
        };

        div()
            .id("conflict_resolver_preview")
            .relative()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .p_2()
            .bg(theme.colors.window_bg)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .w_full()
                    .gap_2()
                    .pr(scrollbar_gutter)
                    .child(self.render_conflict_markdown_preview_column(
                        theme,
                        ThreeWayColumn::Base,
                        cx,
                    ))
                    .child(self.render_conflict_markdown_preview_column(
                        theme,
                        ThreeWayColumn::Ours,
                        cx,
                    ))
                    .child(self.render_conflict_markdown_preview_column(
                        theme,
                        ThreeWayColumn::Theirs,
                        cx,
                    )),
            )
            .when(vertical_sync_enabled, |d| {
                d.child(
                    components::Scrollbar::new(
                        "conflict_markdown_preview_scrollbar",
                        vertical_handle,
                    )
                    .always_visible()
                    .render(theme),
                )
            })
            .into_any_element()
    }

    fn render_conflict_markdown_preview_column(
        &mut self,
        theme: AppTheme,
        side: ThreeWayColumn,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let (id, list_id, vscrollbar_id, hscrollbar_id, label, scroll) = match side {
            ThreeWayColumn::Base => (
                "conflict_preview_base",
                "conflict_preview_base_list",
                "conflict_preview_base_scrollbar",
                "conflict_preview_base_hscrollbar",
                "Base (A)",
                self.conflict_resolver_diff_scroll.clone(),
            ),
            ThreeWayColumn::Ours => (
                "conflict_preview_ours",
                "conflict_preview_ours_list",
                "conflict_preview_ours_scrollbar",
                "conflict_preview_ours_hscrollbar",
                "Local (B)",
                self.conflict_preview_ours_scroll.clone(),
            ),
            ThreeWayColumn::Theirs => (
                "conflict_preview_theirs",
                "conflict_preview_theirs_list",
                "conflict_preview_theirs_scrollbar",
                "conflict_preview_theirs_hscrollbar",
                "Remote (C)",
                self.conflict_preview_theirs_scroll.clone(),
            ),
        };
        let vertical_sync_enabled = self.diff_scroll_sync.includes_vertical();
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
                let list = uniform_list(list_id, $document.rows.len(), cx.processor($processor))
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(&scroll)
                    .with_horizontal_sizing_behavior(
                        gpui::ListHorizontalSizingBehavior::Unconstrained,
                    );
                let vertical_scrollbar_gutter = if vertical_sync_enabled {
                    px(0.0)
                } else {
                    components::Scrollbar::visible_gutter(
                        scroll.clone(),
                        components::ScrollbarAxis::Vertical,
                    )
                };
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        div()
                            .h_full()
                            .min_h(px(0.0))
                            .pr(vertical_scrollbar_gutter)
                            .child(list),
                    )
                    .when(!vertical_sync_enabled, |d| {
                        d.child(
                            components::Scrollbar::new(vscrollbar_id, scroll.clone())
                                .always_visible()
                                .render(theme),
                        )
                    })
                    .child(
                        components::Scrollbar::horizontal(hscrollbar_id, scroll.clone())
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
