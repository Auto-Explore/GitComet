use super::*;

mod diff;
mod history;
mod status_nav;

impl MainPaneView {
    pub(in super::super) fn diff_view(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let repo_id = self.active_repo_id();

        // Intentionally no outer panel header; keep diff controls in the inner header.

        let title: AnyElement = self
            .active_repo()
            .and_then(|r| r.diff_target.as_ref())
            .map(|t| {
                let (icon, color, text): (Option<&'static str>, gpui::Rgba, SharedString) = match t
                {
                    DiffTarget::WorkingTree { path, area } => {
                        let kind = self.active_repo().and_then(|repo| match &repo.status {
                            Loadable::Ready(status) => {
                                let list = match area {
                                    DiffArea::Unstaged => &status.unstaged,
                                    DiffArea::Staged => &status.staged,
                                };
                                list.iter().find(|e| e.path == *path).map(|e| e.kind)
                            }
                            _ => None,
                        });

                        let (icon, color) = match kind.unwrap_or(FileStatusKind::Modified) {
                            FileStatusKind::Untracked | FileStatusKind::Added => {
                                ("+", theme.colors.success)
                            }
                            FileStatusKind::Modified => ("✎", theme.colors.warning),
                            FileStatusKind::Deleted => ("−", theme.colors.danger),
                            FileStatusKind::Renamed => ("→", theme.colors.accent),
                            FileStatusKind::Conflicted => ("!", theme.colors.danger),
                        };
                        (Some(icon), color, self.cached_path_display(path))
                    }
                    DiffTarget::Commit { commit_id: _, path } => match path {
                        Some(path) => (
                            Some("✎"),
                            theme.colors.text_muted,
                            self.cached_path_display(path),
                        ),
                        None => (Some("✎"), theme.colors.text_muted, "Full diff".into()),
                    },
                };

                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .w(px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when_some(icon, |this, icon| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(color)
                                        .child(icon),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(text),
                    )
                    .into_any_element()
            })
            .unwrap_or_else(|| {
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child("Select a file to view diff")
                    .into_any_element()
            });

        let untracked_preview_path = self.untracked_worktree_preview_path();
        let added_preview_path = self.added_file_preview_abs_path();
        let deleted_preview_path = self.deleted_file_preview_abs_path();

        let preview_path = untracked_preview_path
            .as_deref()
            .or(added_preview_path.as_deref())
            .or(deleted_preview_path.as_deref());
        let is_file_preview = preview_path
            .is_some_and(|p| !super::super::should_bypass_text_file_preview_for_path(p));

        if is_file_preview {
            if let Some(path) = untracked_preview_path.clone() {
                self.ensure_worktree_preview_loaded(path, cx);
            } else if let Some(path) = added_preview_path.clone().or(deleted_preview_path.clone()) {
                self.ensure_preview_loading(path);
            }
        }
        let wants_file_diff = !is_file_preview
            && self
                .active_repo()
                .is_some_and(|r| Self::is_file_diff_target(r.diff_target.as_ref()));

        let repo = self.active_repo();
        let conflict_target = repo.and_then(|repo| {
            let DiffTarget::WorkingTree { path, area } = repo.diff_target.as_ref()? else {
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
        let conflict_strategy = Self::conflict_resolver_strategy(conflict_kind);
        let is_conflict_resolver = conflict_strategy.is_some();
        let is_conflict_compare = conflict_target_path.is_some() && conflict_strategy.is_none();

        let diff_target_path = repo.and_then(|repo| match repo.diff_target.as_ref()? {
            DiffTarget::WorkingTree { path, .. } => Some(path.as_path()),
            DiffTarget::Commit {
                path: Some(path), ..
            } => Some(path.as_path()),
            _ => None,
        });
        let is_svg_diff_target = diff_target_path.is_some_and(super::super::is_svg_path);
        let show_svg_view_toggle = wants_file_diff && is_svg_diff_target;
        let is_image_diff_loaded =
            repo.is_some_and(|repo| !matches!(repo.diff_file_image, Loadable::NotLoaded));
        let is_image_diff_view = wants_file_diff
            && is_image_diff_loaded
            && (!is_svg_diff_target || self.svg_diff_view_mode == SvgDiffViewMode::Image);

        let diff_nav_hotkey_hint = |label: &'static str| {
            div()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(label)
        };

        let (prev_file_btn, next_file_btn) = (|| {
            let repo_id = repo_id?;
            let repo = self.active_repo()?;
            let DiffTarget::WorkingTree { path, area } = repo.diff_target.as_ref()? else {
                return None;
            };
            let area = *area;

            let (prev, next) = match &repo.status {
                Loadable::Ready(status) => {
                    let entries = match area {
                        DiffArea::Unstaged => status.unstaged.as_slice(),
                        DiffArea::Staged => status.staged.as_slice(),
                    };
                    Self::status_prev_next_indices(entries, path.as_path())
                }
                _ => (None, None),
            };

            let prev_disabled = prev.is_none();
            let next_disabled = next.is_none();

            let prev_tooltip: SharedString = "Previous file (F1)".into();
            let next_tooltip: SharedString = "Next file (F4)".into();

            let prev_btn = zed::Button::new("diff_prev_file", "Prev file")
                .end_slot(diff_nav_hotkey_hint("F1"))
                .style(zed::ButtonStyle::Outlined)
                .disabled(prev_disabled)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    if this.try_select_adjacent_status_file(repo_id, -1, window, cx) {
                        cx.notify();
                    }
                })
                .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                    let mut changed = false;
                    if *hovering {
                        changed |= this.set_tooltip_text_if_changed(Some(prev_tooltip.clone()), cx);
                    } else {
                        changed |= this.clear_tooltip_if_matches(&prev_tooltip, cx);
                    }
                    if changed {
                        cx.notify();
                    }
                }));

            let next_btn = zed::Button::new("diff_next_file", "Next file")
                .end_slot(diff_nav_hotkey_hint("F4"))
                .style(zed::ButtonStyle::Outlined)
                .disabled(next_disabled)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    if this.try_select_adjacent_status_file(repo_id, 1, window, cx) {
                        cx.notify();
                    }
                })
                .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                    let mut changed = false;
                    if *hovering {
                        changed |= this.set_tooltip_text_if_changed(Some(next_tooltip.clone()), cx);
                    } else {
                        changed |= this.clear_tooltip_if_matches(&next_tooltip, cx);
                    }
                    if changed {
                        cx.notify();
                    }
                }));

            Some((prev_btn, next_btn))
        })()
        .map(|(prev, next)| (Some(prev), Some(next)))
        .unwrap_or((None, None));

        let mut controls = div().flex().items_center().gap_1();
        if is_conflict_resolver {
            let nav_entries = self.conflict_nav_entries();
            let current_nav_ix = self.conflict_resolver.nav_anchor.unwrap_or(0);
            let can_nav_prev =
                diff_navigation::diff_nav_prev_target(&nav_entries, current_nav_ix).is_some();
            let can_nav_next =
                diff_navigation::diff_nav_next_target(&nav_entries, current_nav_ix).is_some();

            controls = controls
                .when_some(prev_file_btn, |d, btn| d.child(btn))
                .child(
                    zed::Button::new("conflict_prev", "Prev")
                        .end_slot(diff_nav_hotkey_hint("F2"))
                        .style(zed::ButtonStyle::Outlined)
                        .disabled(!can_nav_prev)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.conflict_jump_prev();
                            cx.notify();
                        }),
                )
                .child(
                    zed::Button::new("conflict_next", "Next")
                        .end_slot(diff_nav_hotkey_hint("F3"))
                        .style(zed::ButtonStyle::Outlined)
                        .disabled(!can_nav_next)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.conflict_jump_next();
                            cx.notify();
                        }),
                )
                .when_some(next_file_btn, |d, btn| d.child(btn));

            // Show warning if resolved text still contains conflict markers.
            let resolved_has_markers = self
                .conflict_resolved_preview_lines
                .iter()
                .any(|line| {
                    line.starts_with("<<<<<<<")
                        || line.starts_with(">>>>>>>")
                        || line.starts_with("=======")
                        || line.starts_with("|||||||")
                });
            if resolved_has_markers {
                controls = controls.child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.danger)
                        .child("markers remain"),
                );
            }

            if let (Some(repo_id), Some(path)) = (repo_id, conflict_target_path.clone()) {
                let save_path = path.clone();
                controls = controls
                    .child(
                        zed::Button::new("conflict_save", "Save")
                            .style(zed::ButtonStyle::Outlined)
                            .on_click(theme, cx, move |this, _e, _w, cx| {
                                let text = this
                                    .conflict_resolver_input
                                    .read_with(cx, |i, _| i.text().to_string());
                                this.store.dispatch(Msg::SaveWorktreeFile {
                                    repo_id,
                                    path: save_path.clone(),
                                    contents: text,
                                    stage: false,
                                });
                            }),
                    )
                    .child({
                        let save_path = path.clone();
                        zed::Button::new("conflict_save_stage", "Save & stage")
                            .style(zed::ButtonStyle::Filled)
                            .on_click(theme, cx, move |this, e, window, cx| {
                                let text = this
                                    .conflict_resolver_input
                                    .read_with(cx, |i, _| i.text().to_string());
                                if conflict_resolver::text_contains_conflict_markers(&text) {
                                    this.open_popover_at(
                                        PopoverKind::ConflictSaveStageConfirm {
                                            repo_id,
                                            path: save_path.clone(),
                                        },
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                } else {
                                    this.store.dispatch(Msg::SaveWorktreeFile {
                                        repo_id,
                                        path: save_path.clone(),
                                        contents: text,
                                        stage: true,
                                    });
                                }
                            })
                    });
            }
        } else if !is_file_preview {
            controls = controls.when_some(prev_file_btn, |d, btn| d.child(btn));

            if !is_image_diff_view {
                let nav_entries = self.diff_nav_entries();
                let current_nav_ix = self.diff_selection_anchor.unwrap_or(0);
                let can_nav_prev =
                    diff_navigation::diff_nav_prev_target(&nav_entries, current_nav_ix).is_some();
                let can_nav_next =
                    diff_navigation::diff_nav_next_target(&nav_entries, current_nav_ix).is_some();

                let prev_hunk_btn = zed::Button::new("diff_prev_hunk", "Prev")
                    .end_slot(diff_nav_hotkey_hint("F2"))
                    .style(zed::ButtonStyle::Outlined)
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

                let next_hunk_btn = zed::Button::new("diff_next_hunk", "Next")
                    .end_slot(diff_nav_hotkey_hint("F3"))
                    .style(zed::ButtonStyle::Outlined)
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
                let diff_inline_btn = zed::Button::new("diff_inline", "Inline")
                    .borderless()
                    .style(zed::ButtonStyle::Subtle)
                    .selected(self.diff_view == DiffViewMode::Inline)
                    .selected_bg(view_toggle_selected_bg)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.diff_view = DiffViewMode::Inline;
                        this.diff_text_segments_cache.clear();
                        if this.diff_search_active && !this.diff_search_query.as_ref().trim().is_empty()
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

                let diff_split_btn = zed::Button::new("diff_split", "Split")
                    .borderless()
                    .style(zed::ButtonStyle::Subtle)
                    .selected(self.diff_view == DiffViewMode::Split)
                    .selected_bg(view_toggle_selected_bg)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.diff_view = DiffViewMode::Split;
                        this.diff_text_segments_cache.clear();
                        if this.diff_search_active && !this.diff_search_query.as_ref().trim().is_empty()
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
                    .h(px(zed::CONTROL_HEIGHT_PX))
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
                            zed::Button::new("diff_hunks", "Hunks")
                                .style(zed::ButtonStyle::Outlined)
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

            if show_svg_view_toggle {
                controls = controls
                    .child(
                        zed::Button::new("svg_diff_view_image", "Image")
                            .style(if self.svg_diff_view_mode == SvgDiffViewMode::Image {
                                zed::ButtonStyle::Filled
                            } else {
                                zed::ButtonStyle::Outlined
                            })
                            .on_click(theme, cx, |this, _e, _w, cx| {
                                this.svg_diff_view_mode = SvgDiffViewMode::Image;
                                cx.notify();
                            }),
                    )
                    .child(
                        zed::Button::new("svg_diff_view_code", "Code")
                            .style(if self.svg_diff_view_mode == SvgDiffViewMode::Code {
                                zed::ButtonStyle::Filled
                            } else {
                                zed::ButtonStyle::Outlined
                            })
                            .on_click(theme, cx, |this, _e, _w, cx| {
                                this.svg_diff_view_mode = SvgDiffViewMode::Code;
                                cx.notify();
                            }),
                    );
            }
        } else {
            controls = controls
                .when_some(prev_file_btn, |d, btn| d.child(btn))
                .when_some(next_file_btn, |d, btn| d.child(btn));
        }

        // Show-whitespace toggle (Alt+W).
        {
            let ws_label = if self.show_whitespace {
                "WS"
            } else {
                "WS"
            };
            let ws_style = if self.show_whitespace {
                zed::ButtonStyle::Filled
            } else {
                zed::ButtonStyle::Outlined
            };
            controls = controls.child(
                zed::Button::new("show_whitespace", ws_label)
                    .style(ws_style)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.show_whitespace = !this.show_whitespace;
                        this.diff_text_segments_cache.clear();
                        this.conflict_diff_segments_cache_split.clear();
                        this.conflict_diff_segments_cache_inline.clear();
                        this.conflict_three_way_segments_cache.clear();
                        cx.notify();
                    })
                    .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                        let text: SharedString = "Show whitespace (Alt+W)".into();
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

        if let Some(repo_id) = repo_id {
            controls = controls.child(
                zed::Button::new("diff_close", "✕")
                    .style(zed::ButtonStyle::Transparent)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        this.store.dispatch(Msg::ClearDiffSelection { repo_id });
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
                    zed::Button::new("diff_search_close", "✕")
                        .style(zed::ButtonStyle::Transparent)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.diff_search_active = false;
                            this.diff_search_matches.clear();
                            this.diff_search_match_ix = None;
                            this.diff_text_segments_cache.clear();
                            this.worktree_preview_segments_cache_path = None;
                            this.worktree_preview_segments_cache.clear();
                            this.conflict_diff_segments_cache_split.clear();
                            this.conflict_diff_segments_cache_inline.clear();
                            window.focus(&this.diff_panel_focus_handle);
                            cx.notify();
                        }),
                );
        }

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(zed::CONTROL_HEIGHT_MD_PX))
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

        let body: AnyElement = if is_file_preview {
            if added_preview_path.is_some() || deleted_preview_path.is_some() {
                self.try_populate_worktree_preview_from_diff_file();
            }
            match &self.worktree_preview {
                Loadable::NotLoaded | Loadable::Loading => {
                    zed::empty_state(theme, "File", "Loading").into_any_element()
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
                Loadable::Ready(lines) => {
                    if lines.is_empty() {
                        zed::empty_state(theme, "File", "Empty file.").into_any_element()
                    } else {
                        let list = uniform_list(
                            "worktree_preview_list",
                            lines.len(),
                            cx.processor(Self::render_worktree_preview_rows),
                        )
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(self.worktree_preview_scroll.clone());

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
                                zed::Scrollbar::new("worktree_preview_scrollbar", scroll_handle)
                                    .render(theme),
                            )
                            .into_any_element()
                    }
                }
            }
        } else if is_conflict_resolver {
            match (repo, conflict_target_path) {
                (None, _) => {
                    zed::empty_state(theme, "Resolve", "No repository.").into_any_element()
                }
                (_, None) => zed::empty_state(theme, "Resolve", "No conflicted file selected.")
                    .into_any_element(),
                (Some(repo), Some(path)) => {
                    let title: SharedString =
                        format!("Resolve conflict: {}", self.cached_path_display(&path)).into();

                    match &repo.conflict_file {
                        Loadable::NotLoaded | Loadable::Loading => {
                            zed::empty_state(theme, title, "Loading conflict data…")
                                .into_any_element()
                        }
                        Loadable::Error(e) => {
                            zed::empty_state(theme, title, e.clone()).into_any_element()
                        }
                        Loadable::Ready(None) => {
                            zed::empty_state(theme, title, "No conflict data.").into_any_element()
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

                            let clear_selection =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_clear_selection(cx)
                                };

                            let append_selection =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_append_selection_to_output(cx);
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

                            let base_for_btn = base.clone();
                            let set_output_base =
                                move |this: &mut Self,
                                      _e: &ClickEvent,
                                      _w: &mut Window,
                                      cx: &mut gpui::Context<Self>| {
                                    if this.conflict_resolver_conflict_count() > 0 {
                                        this.conflict_resolver_pick_active_conflict(
                                            conflict_resolver::ConflictChoice::Base,
                                            cx,
                                        );
                                    } else {
                                        this.conflict_resolver_set_output(
                                            base_for_btn.clone(),
                                            cx,
                                        );
                                    }
                                };
                            let local_for_btn = local.clone();
                            let set_output_local =
                                move |this: &mut Self,
                                      _e: &ClickEvent,
                                      _w: &mut Window,
                                      cx: &mut gpui::Context<Self>| {
                                    if this.conflict_resolver_conflict_count() > 0 {
                                        this.conflict_resolver_pick_active_conflict(
                                            conflict_resolver::ConflictChoice::Ours,
                                            cx,
                                        );
                                    } else {
                                        this.conflict_resolver_set_output(
                                            local_for_btn.clone(),
                                            cx,
                                        );
                                    }
                                };
                            let remote_for_btn = remote.clone();
                            let set_output_remote =
                                move |this: &mut Self,
                                      _e: &ClickEvent,
                                      _w: &mut Window,
                                      cx: &mut gpui::Context<Self>| {
                                    if this.conflict_resolver_conflict_count() > 0 {
                                        this.conflict_resolver_pick_active_conflict(
                                            conflict_resolver::ConflictChoice::Theirs,
                                            cx,
                                        );
                                    } else {
                                        this.conflict_resolver_set_output(
                                            remote_for_btn.clone(),
                                            cx,
                                        );
                                    }
                                };
                            let reset_from_markers =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_reset_output_from_markers(cx);
                                };

                            let pick_all_base =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_pick_all_conflicts(
                                        conflict_resolver::ConflictChoice::Base,
                                        cx,
                                    );
                                };
                            let pick_all_local =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_pick_all_conflicts(
                                        conflict_resolver::ConflictChoice::Ours,
                                        cx,
                                    );
                                };
                            let pick_all_remote =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_pick_all_conflicts(
                                        conflict_resolver::ConflictChoice::Theirs,
                                        cx,
                                    );
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

                            let view_mode_controls = div()
                                .id("conflict_view_mode_toggle")
                                .flex()
                                .items_center()
                                .h(px(zed::CONTROL_HEIGHT_PX))
                                .rounded(px(theme.radii.row))
                                .border_1()
                                .border_color(view_toggle_border)
                                .bg(gpui::rgba(0x00000000))
                                .overflow_hidden()
                                .p(px(1.0))
                                .child(
                                    zed::Button::new("conflict_view_three_way", "3-way")
                                        .borderless()
                                        .style(zed::ButtonStyle::Subtle)
                                        .selected(view_mode == ConflictResolverViewMode::ThreeWay)
                                        .selected_bg(view_toggle_selected_bg)
                                        .on_click(theme, cx, set_view_three_way),
                                )
                                .child(
                                    div().h_full().w(px(1.0)).bg(view_toggle_divider),
                                )
                                .child(
                                    zed::Button::new("conflict_view_two_way", "2-way")
                                        .borderless()
                                        .style(zed::ButtonStyle::Subtle)
                                        .selected(
                                            view_mode == ConflictResolverViewMode::TwoWayDiff,
                                        )
                                        .selected_bg(view_toggle_selected_bg)
                                        .on_click(theme, cx, set_view_two_way),
                                );

                            let diff_len = match view_mode {
                                ConflictResolverViewMode::ThreeWay => {
                                    self.conflict_resolver.three_way_len
                                }
                                ConflictResolverViewMode::TwoWayDiff => match mode {
                                    ConflictDiffMode::Split => {
                                        self.conflict_resolver.diff_rows.len()
                                    }
                                    ConflictDiffMode::Inline => {
                                        self.conflict_resolver.inline_rows.len()
                                    }
                                },
                            };

                            let selection_empty = view_mode == ConflictResolverViewMode::ThreeWay
                                || self.conflict_resolver_selection_is_empty();

                            let mode_controls = div()
                                .id("conflict_mode_toggle")
                                .flex()
                                .items_center()
                                .h(px(zed::CONTROL_HEIGHT_PX))
                                .rounded(px(theme.radii.row))
                                .border_1()
                                .border_color(view_toggle_border)
                                .bg(gpui::rgba(0x00000000))
                                .overflow_hidden()
                                .p(px(1.0))
                                .child(
                                    zed::Button::new("conflict_mode_split", "Split")
                                        .borderless()
                                        .style(zed::ButtonStyle::Subtle)
                                        .selected(mode == ConflictDiffMode::Split)
                                        .selected_bg(view_toggle_selected_bg)
                                        .on_click(theme, cx, toggle_mode_split),
                                )
                                .child(
                                    div().h_full().w(px(1.0)).bg(view_toggle_divider),
                                )
                                .child(
                                    zed::Button::new("conflict_mode_inline", "Inline")
                                        .borderless()
                                        .style(zed::ButtonStyle::Subtle)
                                        .selected(mode == ConflictDiffMode::Inline)
                                        .selected_bg(view_toggle_selected_bg)
                                        .on_click(theme, cx, toggle_mode_inline),
                                );

                            let selection_controls = div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    zed::Button::new(
                                        "conflict_append_selected",
                                        "Append selection",
                                    )
                                    .style(zed::ButtonStyle::Outlined)
                                    .disabled(selection_empty)
                                    .on_click(
                                        theme,
                                        cx,
                                        append_selection,
                                    ),
                                )
                                .child(
                                    zed::Button::new("conflict_clear_selected", "Clear selection")
                                        .style(zed::ButtonStyle::Transparent)
                                        .disabled(selection_empty)
                                        .on_click(theme, cx, clear_selection),
                                );

                            let conflict_count = self.conflict_resolver_conflict_count();
                            let active_conflict = self.conflict_resolver.active_conflict;
                            let has_conflicts = conflict_count > 0;
                            let resolved_count = self.conflict_resolver_resolved_count();
                            let unresolved_count = conflict_count - resolved_count;

                            let any_block_missing_base = has_conflicts
                                && self
                                    .conflict_resolver
                                    .marker_segments
                                    .iter()
                                    .any(|seg| {
                                        matches!(
                                            seg,
                                            conflict_resolver::ConflictSegment::Block(b)
                                                if b.base.is_none()
                                        )
                                    });

                            let active_block_has_base = if has_conflicts {
                                let mut seen = 0usize;
                                self.conflict_resolver
                                    .marker_segments
                                    .iter()
                                    .find_map(|seg| {
                                        let conflict_resolver::ConflictSegment::Block(block) = seg
                                        else {
                                            return None;
                                        };
                                        let hit = seen == active_conflict;
                                        seen += 1;
                                        hit.then_some(block.base.is_some())
                                    })
                                    .unwrap_or(false)
                            } else {
                                file.base.is_some()
                            };

                            let prev_conflict =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_prev_conflict(cx);
                                };
                            let next_conflict =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_next_conflict(cx);
                                };

                            let auto_resolve =
                                |this: &mut Self,
                                 _e: &ClickEvent,
                                 _w: &mut Window,
                                 cx: &mut gpui::Context<Self>| {
                                    this.conflict_resolver_auto_resolve(cx);
                                };

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
                                    let resolved_label: SharedString = format!(
                                        "Resolved {}/{}",
                                        resolved_count,
                                        conflict_count
                                    )
                                    .into();
                                    d.child(
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
                                    )
                                    .child(
                                        zed::Button::new("conflict_pick_prev", "Prev")
                                            .style(zed::ButtonStyle::Transparent)
                                            .disabled(active_conflict == 0)
                                            .on_click(theme, cx, prev_conflict),
                                    )
                                    .child(
                                        zed::Button::new("conflict_pick_next", "Next")
                                            .style(zed::ButtonStyle::Transparent)
                                            .disabled(active_conflict + 1 >= conflict_count)
                                            .on_click(theme, cx, next_conflict),
                                    )
                                })
                                .child(
                                    zed::Button::new("conflict_use_base", "A (base)")
                                        .style(zed::ButtonStyle::Transparent)
                                        .disabled(if has_conflicts {
                                            !active_block_has_base
                                        } else {
                                            file.base.is_none()
                                        })
                                        .on_click(theme, cx, set_output_base),
                                )
                                .child(
                                    zed::Button::new("conflict_use_local", "B (local)")
                                        .style(zed::ButtonStyle::Transparent)
                                        .disabled(!has_conflicts && file.ours.is_none())
                                        .on_click(theme, cx, set_output_local),
                                )
                                .child(
                                    zed::Button::new("conflict_use_remote", "C (remote)")
                                        .style(zed::ButtonStyle::Transparent)
                                        .disabled(!has_conflicts && file.theirs.is_none())
                                        .on_click(theme, cx, set_output_remote),
                                )
                                .when(has_conflicts && conflict_count > 1, |d| {
                                    d.child(
                                        div()
                                            .w(px(1.0))
                                            .h(px(12.0))
                                            .bg(theme.colors.border),
                                    )
                                    .child(
                                        zed::Button::new("conflict_all_base", "All → A")
                                            .style(zed::ButtonStyle::Transparent)
                                            .disabled(any_block_missing_base)
                                            .on_click(theme, cx, pick_all_base),
                                    )
                                    .child(
                                        zed::Button::new("conflict_all_local", "All → B")
                                            .style(zed::ButtonStyle::Transparent)
                                            .on_click(theme, cx, pick_all_local),
                                    )
                                    .child(
                                        zed::Button::new("conflict_all_remote", "All → C")
                                            .style(zed::ButtonStyle::Transparent)
                                            .on_click(theme, cx, pick_all_remote),
                                    )
                                })
                                .child(
                                    zed::Button::new(
                                        "conflict_reset_markers",
                                        "Reset from markers",
                                    )
                                    .style(zed::ButtonStyle::Transparent)
                                    .disabled(!has_current)
                                    .on_click(
                                        theme,
                                        cx,
                                        reset_from_markers,
                                    ),
                                )
                                .when(has_conflicts && unresolved_count > 0, |d| {
                                    d.child(
                                        div()
                                            .w(px(1.0))
                                            .h(px(12.0))
                                            .bg(theme.colors.border),
                                    )
                                    .child(
                                        zed::Button::new(
                                            "conflict_auto_resolve",
                                            "Auto-resolve safe",
                                        )
                                        .style(zed::ButtonStyle::Outlined)
                                        .on_click(theme, cx, auto_resolve),
                                    )
                                });

                            let top_header = div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(div().text_xs().text_color(theme.colors.text_muted).child(
                                    match view_mode {
                                        ConflictResolverViewMode::ThreeWay => {
                                            "Merge inputs (base / local / remote)"
                                        }
                                        ConflictResolverViewMode::TwoWayDiff => {
                                            "Diff (local ↔ remote)"
                                        }
                                    },
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(view_mode_controls)
                                        .when(
                                            view_mode == ConflictResolverViewMode::TwoWayDiff,
                                            |d| d.child(mode_controls).child(selection_controls),
                                        ),
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
                                (available * ratios[0]).max(min_col_w).min(available - min_col_w * 2.0)
                            };
                            let col_b_w = if available <= min_col_w * 3.0 {
                                available / 3.0
                            } else {
                                (available * (ratios[1] - ratios[0])).max(min_col_w).min(available - col_a_w - min_col_w)
                            };
                            let col_c_w = (available - col_a_w - col_b_w).max(px(0.0));
                            self.conflict_three_way_col_widths = [col_a_w, col_b_w, col_c_w];

                            // Compute two-way diff split column widths
                            {
                                let two_available = (main_w - handle_w).max(px(0.0));
                                let two_ratio = self.conflict_diff_split_ratio;
                                let left_w = (two_available * two_ratio)
                                    .max(min_col_w)
                                    .min(two_available - min_col_w);
                                let right_w = two_available - left_w;
                                self.conflict_diff_split_col_widths = [left_w, right_w];
                            }

                            let conflict_hsplit_resize_handle = |id: &'static str, which: ConflictHSplitResizeHandle| {
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
                                        which,
                                        |_handle, _offset, _window, cx| {
                                            cx.new(|_cx| ConflictHSplitResizeDragGhost)
                                        },
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, e: &MouseDownEvent, _w, cx| {
                                                cx.stop_propagation();
                                                this.conflict_hsplit_resize = Some(
                                                    ConflictHSplitResizeState {
                                                        handle: which,
                                                        start_x: e.position.x,
                                                        start_ratios: this.conflict_three_way_col_ratios,
                                                    },
                                                );
                                                cx.notify();
                                            },
                                        ),
                                    )
                                    .on_drag_move(cx.listener(
                                        move |this,
                                              e: &gpui::DragMoveEvent<ConflictHSplitResizeHandle>,
                                              _w,
                                              cx| {
                                            let Some(state) = this.conflict_hsplit_resize else {
                                                return;
                                            };
                                            if state.handle != *e.drag(cx) {
                                                return;
                                            }

                                            let main_w = this.main_pane_content_width(cx);
                                            let avail = (main_w - handle_w * 2.0).max(px(0.0));
                                            if avail <= min_col_w * 3.0 {
                                                this.conflict_three_way_col_ratios = [1.0 / 3.0, 2.0 / 3.0];
                                                cx.notify();
                                                return;
                                            }

                                            let dx = e.event.position.x - state.start_x;
                                            let mut r = state.start_ratios;
                                            match state.handle {
                                                ConflictHSplitResizeHandle::First => {
                                                    let new_pos = (avail * r[0] + dx).max(min_col_w).min(avail - min_col_w * 2.0 - handle_w);
                                                    r[0] = (new_pos / avail).clamp(0.0, 1.0);
                                                    // Ensure second divider stays valid
                                                    let min_r1 = r[0] + (min_col_w / avail);
                                                    if r[1] < min_r1 {
                                                        r[1] = min_r1.min(1.0 - min_col_w / avail);
                                                    }
                                                }
                                                ConflictHSplitResizeHandle::Second => {
                                                    let new_pos = (avail * r[1] + dx).max(min_col_w * 2.0 + handle_w).min(avail - min_col_w);
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
                                                        let main_w_val =
                                                            this.conflict_diff_split_col_widths[0]
                                                                + this
                                                                    .conflict_diff_split_col_widths
                                                                    [1]
                                                                + px(PANE_RESIZE_HANDLE_PX);
                                                        if main_w_val <= px(0.0) {
                                                            return;
                                                        }
                                                        let dx =
                                                            e.event.position.x - state.start_x;
                                                        let delta = dx / main_w_val;
                                                        let new_ratio = (state.start_ratio
                                                            + delta)
                                                            .clamp(0.1, 0.9);
                                                        this.conflict_diff_split_ratio = new_ratio;
                                                        this.conflict_diff_segments_cache_split
                                                            .clear();
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
                                zed::empty_state(theme, "Inputs", "Stage data not available.")
                                    .into_any_element()
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

                                let scroll_handle = self
                                    .conflict_resolver_diff_scroll
                                    .0
                                    .borrow()
                                    .base_handle
                                    .clone();

                                div()
                                    .id("conflict_resolver_diff_scroll")
                                    .relative()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .bg(theme.colors.window_bg)
                                    .child(list)
                                    .child(
                                        zed::Scrollbar::new(
                                            "conflict_resolver_diff_scrollbar",
                                            scroll_handle,
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
                                        let Some(state) = this.conflict_resolver_vsplit_resize else {
                                            return;
                                        };

                                        let total_h = this.last_window_size.height;
                                        // Approximate available height (window - chrome)
                                        let available = (total_h - px(200.0)).max(min_section_h * 2.0);
                                        let dy = e.event.position.y - state.start_y;
                                        let mut next_top = (available * state.start_ratio) + dy;
                                        next_top = next_top.max(min_section_h).min(available - min_section_h);
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
                        .child({
                            let mut bottom_section = div()
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
                                    div()
                                        .id("conflict_resolver_output_scroll")
                                        .flex_1()
                                        .min_h(px(0.0))
                                        .overflow_y_scroll()
                                        .child(
                                            div()
                                                .p_2()
                                                .child(
                                                    self.conflict_resolver_input.clone(),
                                                ),
                                        ),
                                );
                            bottom_section.style().flex_grow = Some(1.0 - vsplit_ratio);
                            bottom_section.style().flex_shrink = Some(1.0);
                            bottom_section.style().flex_basis = Some(relative(0.).into());
                            bottom_section
                        })
                        .into_any_element()
                        }
                    }
                }
            }
        } else if is_conflict_compare {
            match (repo, conflict_target_path) {
                (None, _) => {
                    zed::empty_state(theme, "Resolve", "No repository.").into_any_element()
                }
                (_, None) => zed::empty_state(theme, "Resolve", "No conflicted file selected.")
                    .into_any_element(),
                (Some(repo), Some(path)) => {
                    let title: SharedString =
                        format!("Resolve conflict: {}", self.cached_path_display(&path)).into();

                    match &repo.conflict_file {
                        Loadable::NotLoaded | Loadable::Loading => {
                            zed::empty_state(theme, title, "Loading conflict data…")
                                .into_any_element()
                        }
                        Loadable::Error(e) => {
                            zed::empty_state(theme, title, e.clone()).into_any_element()
                        }
                        Loadable::Ready(None) => {
                            zed::empty_state(theme, title, "No conflict data.").into_any_element()
                        }
                        Loadable::Ready(Some(file)) => {
                            if file.path != path {
                                zed::empty_state(theme, title, "Loading conflict data…")
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

                                let columns_header =
                                    zed::split_columns_header(theme, ours_label, theirs_label);

                                let diff_len = match self.diff_view {
                                    DiffViewMode::Split => self.conflict_resolver.diff_rows.len(),
                                    DiffViewMode::Inline => {
                                        self.conflict_resolver.inline_rows.len()
                                    }
                                };

                                let diff_body: AnyElement = if diff_len == 0 {
                                    zed::empty_state(theme, "Diff", "No conflict diff to show.")
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
                                                    zed::Scrollbar::new(
                                                        "conflict_compare_scrollbar",
                                                        scroll_handle.clone(),
                                                    )
                                                    .always_visible()
                                                    .render(theme),
                                                )
                                                .child(
                                                    zed::Scrollbar::horizontal(
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
                None => zed::empty_state(theme, "Diff", "No repository.").into_any_element(),
                Some(repo) => match &repo.diff {
                    Loadable::NotLoaded => {
                        zed::empty_state(theme, "Diff", "Select a file.").into_any_element()
                    }
                    Loadable::Loading => {
                        zed::empty_state(theme, "Diff", "Loading").into_any_element()
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
                                    || self.diff_cache_rev != repo.diff_rev
                                    || self.diff_cache_target != repo.diff_target
                                    || self.diff_cache.len() != diff.lines.len()
                                {
                                    self.rebuild_diff_cache(cx);
                                }

                                self.ensure_diff_visible_indices();
                                self.maybe_autoscroll_diff_to_first_change();
                                if self.diff_cache.is_empty() {
                                    zed::empty_state(theme, "Diff", "No differences.")
                                        .into_any_element()
                                } else if self.diff_visible_indices.is_empty() {
                                    zed::empty_state(theme, "Diff", "Nothing to render.")
                                        .into_any_element()
                                } else {
                                    let scroll_handle =
                                        self.diff_scroll.0.borrow().base_handle.clone();
                                    let markers = self.diff_scrollbar_markers_cache.clone();
                                    match self.diff_view {
                                        DiffViewMode::Inline => {
                                            let list = uniform_list(
                                                "diff",
                                                self.diff_visible_indices.len(),
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
                                                    zed::Scrollbar::new(
                                                        "diff_scrollbar",
                                                        scroll_handle.clone(),
                                                    )
                                                    .markers(markers)
                                                    .always_visible()
                                                    .render(theme),
                                                )
                                                .child(
                                                    zed::Scrollbar::horizontal(
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
                                            let count = self.diff_visible_indices.len();
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
                                                .h(px(zed::CONTROL_HEIGHT_PX))
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
                                                                    zed::Scrollbar::horizontal(
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
                                                                    zed::Scrollbar::horizontal(
                                                                        "diff_split_right_hscrollbar",
                                                                        right_scroll_handle,
                                                                    )
                                                                    .always_visible()
                                                                    .render(theme),
                                                                ),
                                                        ),
                                                )
                                                .child(
                                                    zed::Scrollbar::new(
                                                        "diff_scrollbar",
                                                        scroll_handle.clone(),
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
                        this.diff_text_segments_cache.clear();
                        this.worktree_preview_segments_cache_path = None;
                        this.worktree_preview_segments_cache.clear();
                        this.conflict_diff_segments_cache_split.clear();
                        this.conflict_diff_segments_cache_inline.clear();
                        window.focus(&this.diff_panel_focus_handle);
                        handled = true;
                    }
                    if !handled && let Some(repo_id) = this.active_repo_id() {
                        this.clear_status_multi_selection(repo_id, cx);
                        this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                        handled = true;
                    }
                }

                if !handled
                    && (mods.control || mods.platform)
                    && !mods.alt
                    && !mods.function
                    && key == "f"
                {
                    this.diff_search_active = true;
                    this.diff_text_segments_cache.clear();
                    this.worktree_preview_segments_cache_path = None;
                    this.worktree_preview_segments_cache.clear();
                    this.conflict_diff_segments_cache_split.clear();
                    this.conflict_diff_segments_cache_inline.clear();
                    this.diff_search_recompute_matches();
                    let focus = this.diff_search_input.read(cx).focus_handle();
                    window.focus(&focus);
                    handled = true;
                }

                if !handled
                    && this.diff_search_active
                    && key == "f2"
                    && !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                {
                    this.diff_search_prev_match();
                    handled = true;
                }

                if !handled
                    && this.diff_search_active
                    && key == "f3"
                    && !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                {
                    this.diff_search_next_match();
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
                    && let Some(DiffTarget::WorkingTree { path, area }) = repo.diff_target.clone()
                {
                    let next_path_in_area = |entries: &[gitgpui_core::domain::FileStatus]| {
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
                                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
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
                                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
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

                let is_file_preview = this.untracked_worktree_preview_path().is_some()
                    || this.added_file_preview_abs_path().is_some()
                    || this.deleted_file_preview_abs_path().is_some();
                if is_file_preview {
                    if handled {
                        cx.stop_propagation();
                        cx.notify();
                    }
                    return;
                }

                let copy_target_is_focused = this
                    .diff_raw_input
                    .read(cx)
                    .focus_handle()
                    .is_focused(window);

                let conflict_resolver_active = this.active_repo().is_some_and(|repo| {
                    let Some(DiffTarget::WorkingTree { path, area }) = repo.diff_target.as_ref()
                    else {
                        return false;
                    };
                    if *area != DiffArea::Unstaged {
                        return false;
                    }
                    let Loadable::Ready(status) = &repo.status else {
                        return false;
                    };
                    let conflict = status.unstaged.iter().find(|e| {
                        e.path == *path
                            && e.kind == gitgpui_core::domain::FileStatusKind::Conflicted
                    });
                    conflict
                        .and_then(|e| Self::conflict_resolver_strategy(e.conflict))
                        .is_some()
                });

                if mods.alt && !mods.control && !mods.platform && !mods.function {
                    match key {
                        "i" => {
                            if conflict_resolver_active {
                                this.conflict_resolver_set_mode(ConflictDiffMode::Inline, cx);
                            } else {
                                this.diff_view = DiffViewMode::Inline;
                                this.diff_text_segments_cache.clear();
                            }
                            handled = true;
                        }
                        "s" => {
                            if conflict_resolver_active {
                                this.conflict_resolver_set_mode(ConflictDiffMode::Split, cx);
                            } else {
                                this.diff_view = DiffViewMode::Split;
                                this.diff_text_segments_cache.clear();
                            }
                            handled = true;
                        }
                        "h" => {
                            let is_file_preview = this.untracked_worktree_preview_path().is_some()
                                || this.added_file_preview_abs_path().is_some()
                                || this.deleted_file_preview_abs_path().is_some();
                            if !is_file_preview
                                && !this.active_repo().is_some_and(|r| {
                                    Self::is_file_diff_target(r.diff_target.as_ref())
                                })
                            {
                                this.open_popover_at_cursor(PopoverKind::DiffHunks, window, cx);
                                handled = true;
                            }
                        }
                        "w" => {
                            this.show_whitespace = !this.show_whitespace;
                            // Clear styled text caches so they rebuild with new whitespace setting.
                            this.diff_text_segments_cache.clear();
                            this.conflict_diff_segments_cache_split.clear();
                            this.conflict_diff_segments_cache_inline.clear();
                            this.conflict_three_way_segments_cache.clear();
                            handled = true;
                        }
                        "up" => {
                            this.diff_jump_prev();
                            handled = true;
                        }
                        "down" => {
                            this.diff_jump_next();
                            handled = true;
                        }
                        _ => {}
                    }
                }

                if !handled
                    && key == "f7"
                    && !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                {
                    if mods.shift {
                        if conflict_resolver_active {
                            this.conflict_jump_prev();
                        } else {
                            this.diff_jump_prev();
                        }
                    } else {
                        if conflict_resolver_active {
                            this.conflict_jump_next();
                        } else {
                            this.diff_jump_next();
                        }
                    }
                    handled = true;
                }

                if !handled
                    && key == "f2"
                    && !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                {
                    if conflict_resolver_active {
                        this.conflict_jump_prev();
                    } else {
                        this.diff_jump_prev();
                    }
                    handled = true;
                }

                if !handled
                    && key == "f3"
                    && !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                {
                    if conflict_resolver_active {
                        this.conflict_jump_next();
                    } else {
                        this.diff_jump_next();
                    }
                    handled = true;
                }

                // A/B/C quick-pick for conflict resolver (kdiff3-style)
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
                {
                    match key {
                        "a" => {
                            this.conflict_resolver_pick_active_conflict(
                                conflict_resolver::ConflictChoice::Base,
                                cx,
                            );
                            handled = true;
                        }
                        "b" => {
                            this.conflict_resolver_pick_active_conflict(
                                conflict_resolver::ConflictChoice::Ours,
                                cx,
                            );
                            handled = true;
                        }
                        "c" => {
                            this.conflict_resolver_pick_active_conflict(
                                conflict_resolver::ConflictChoice::Theirs,
                                cx,
                            );
                            handled = true;
                        }
                        _ => {}
                    }
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
                    .h(px(zed::CONTROL_HEIGHT_MD_PX))
                    .px_2()
                    .bg(theme.colors.surface_bg_elevated)
                    .border_b_1()
                    .border_color(theme.colors.border),
            )
            .child(div().flex_1().min_h(px(0.0)).w_full().h_full().child(body))
            .child(DiffTextSelectionTracker { view: cx.entity() })
    }
}
