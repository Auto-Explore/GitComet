use super::*;
use gitcomet_core::domain::HistoryMode;
use gitcomet_state::model::GitLogTagFetchMode;

/// Window geometry and pane state restored at startup.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct WindowPreferences {
    pub(super) width: Option<u32>,
    pub(super) height: Option<u32>,
    pub(super) sidebar_width: Option<u32>,
    pub(super) details_width: Option<u32>,
    pub(super) sidebar_collapsed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AppearancePreferences {
    pub(super) theme_mode: ThemeMode,
    pub(super) ui_scale_percent: u32,
    pub(super) date_time_format: DateTimeFormat,
    pub(super) timezone: Timezone,
    pub(super) show_timezone: bool,
}

impl Default for AppearancePreferences {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::default(),
            ui_scale_percent: 100,
            date_time_format: DateTimeFormat::YmdHm,
            timezone: Timezone::default(),
            show_timezone: true,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ChangeTrackingPreferences {
    pub(super) view: ChangeTrackingView,
    pub(super) height: Option<u32>,
    pub(super) untracked_height: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DiffPreferences {
    pub(super) scroll_sync: DiffScrollSync,
    pub(super) content_mode: DiffContentMode,
    pub(super) whitespace_mode: DiffWhitespaceMode,
    pub(super) view_mode: DiffViewMode,
    pub(super) annotate_enabled: bool,
    pub(super) reveal_whitespace_chars: bool,
    pub(super) word_wrap: bool,
    pub(super) show_line_numbers: bool,
}

impl Default for DiffPreferences {
    fn default() -> Self {
        Self {
            scroll_sync: DiffScrollSync::default(),
            content_mode: DiffContentMode::default(),
            whitespace_mode: DiffWhitespaceMode::default(),
            view_mode: DiffViewMode::Split,
            annotate_enabled: false,
            reveal_whitespace_chars: false,
            word_wrap: false,
            show_line_numbers: true,
        }
    }
}

/// Whether rendered Markdown may turn repository-controlled HTTP(S) image
/// sources into network requests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum RemoteMarkdownImagePolicy {
    /// Preserve the historical behavior: remote pictures load as soon as the
    /// rendered preview asks for them.
    #[default]
    AlwaysLoad,
    /// Keep remote pictures inert until the user approves their URL in the
    /// current preview target.
    AskBeforeLoading,
    /// Never make a request for a remote Markdown picture.
    NeverLoad,
}

impl RemoteMarkdownImagePolicy {
    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::AlwaysLoad => "always",
            Self::AskBeforeLoading => "ask",
            Self::NeverLoad => "never",
        }
    }

    pub(super) fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "always" => Some(Self::AlwaysLoad),
            "ask" => Some(Self::AskBeforeLoading),
            "never" => Some(Self::NeverLoad),
            _ => None,
        }
    }

    pub(super) const fn settings_label(self) -> &'static str {
        match self {
            Self::AlwaysLoad => "Always load",
            Self::AskBeforeLoading => "Ask before loading",
            Self::NeverLoad => "Never load",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SecurityPreferences {
    pub(super) remote_markdown_images: RemoteMarkdownImagePolicy,
    pub(super) check_for_updates_on_startup: bool,
}

impl Default for SecurityPreferences {
    fn default() -> Self {
        Self {
            remote_markdown_images: RemoteMarkdownImagePolicy::AlwaysLoad,
            check_for_updates_on_startup: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MergeToolPreferences {
    pub(super) auto_advance: bool,
    pub(super) collapse_unchanged: bool,
    pub(super) output_scroll_sync: bool,
    pub(super) show_line_numbers: bool,
    pub(super) view_three_way: bool,
}

impl Default for MergeToolPreferences {
    fn default() -> Self {
        Self {
            auto_advance: true,
            collapse_unchanged: false,
            output_scroll_sync: true,
            show_line_numbers: true,
            view_three_way: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HistoryPreferences {
    pub(super) show_graph: bool,
    pub(super) show_author: bool,
    pub(super) show_date: bool,
    pub(super) show_sha: bool,
    pub(super) relative_dates: bool,
    pub(super) highlight_commit_chain: bool,
    pub(super) show_tags: bool,
    pub(super) tag_fetch_mode: GitLogTagFetchMode,
    pub(super) default_mode: HistoryMode,
}

impl Default for HistoryPreferences {
    fn default() -> Self {
        Self {
            show_graph: true,
            show_author: true,
            show_date: true,
            show_sha: false,
            relative_dates: true,
            highlight_commit_chain: true,
            show_tags: true,
            tag_fetch_mode: GitLogTagFetchMode::default(),
            default_mode: HistoryMode::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct FileEditingPreferences {
    pub(super) auto_save: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct RepositoryPreferences {
    pub(super) commit_push_after_enabled: bool,
    pub(super) default_tag_type: DefaultTagType,
}

/// Parsed, defaulted preferences shared by the root view and its child views.
///
/// The on-disk session remains a backwards-compatible DTO of optional fields;
/// this is the typed runtime model, so parsing and default choices live in one
/// place instead of being repeated by every window.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct UiPreferences {
    pub(super) window: WindowPreferences,
    pub(super) appearance: AppearancePreferences,
    pub(super) change_tracking: ChangeTrackingPreferences,
    pub(super) diff: DiffPreferences,
    pub(super) security: SecurityPreferences,
    pub(super) merge_tool: MergeToolPreferences,
    pub(super) history: HistoryPreferences,
    pub(super) file_editing: FileEditingPreferences,
    pub(super) repository: RepositoryPreferences,
    pub(super) terminal: TerminalPreferences,
}

impl UiPreferences {
    pub(super) fn from_session(session: &session::UiSession) -> Self {
        Self {
            window: WindowPreferences {
                width: session.window_width,
                height: session.window_height,
                sidebar_width: session.sidebar_width,
                details_width: session.details_width,
                sidebar_collapsed: session.sidebar_collapsed.unwrap_or(false),
            },
            appearance: AppearancePreferences {
                theme_mode: session
                    .theme_mode
                    .as_deref()
                    .and_then(ThemeMode::from_key)
                    .unwrap_or_default(),
                ui_scale_percent: ui_scale::sanitize_percent(session.ui_scale_percent),
                date_time_format: session
                    .date_time_format
                    .as_deref()
                    .and_then(DateTimeFormat::from_key)
                    .unwrap_or(DateTimeFormat::YmdHm),
                timezone: session
                    .timezone
                    .as_deref()
                    .and_then(Timezone::from_key)
                    .unwrap_or_default(),
                show_timezone: session.show_timezone.unwrap_or(true),
            },
            change_tracking: ChangeTrackingPreferences {
                view: session
                    .change_tracking_view
                    .as_deref()
                    .and_then(ChangeTrackingView::from_key)
                    .unwrap_or_default(),
                height: session.change_tracking_height,
                untracked_height: session.untracked_height,
            },
            diff: DiffPreferences {
                scroll_sync: session
                    .diff_scroll_sync
                    .as_deref()
                    .and_then(DiffScrollSync::from_key)
                    .unwrap_or_default(),
                content_mode: session
                    .diff_content_mode
                    .as_deref()
                    .and_then(DiffContentMode::from_key)
                    .unwrap_or_default(),
                whitespace_mode: session
                    .diff_whitespace_mode
                    .as_deref()
                    .and_then(DiffWhitespaceMode::from_key)
                    .unwrap_or_default(),
                view_mode: session
                    .diff_view_mode
                    .as_deref()
                    .and_then(DiffViewMode::from_key)
                    .unwrap_or(DiffViewMode::Split),
                annotate_enabled: session.annotate_enabled.unwrap_or(false),
                reveal_whitespace_chars: session.diff_reveal_whitespace_chars.unwrap_or(false),
                word_wrap: session.diff_word_wrap.unwrap_or(false),
                show_line_numbers: session.diff_show_line_numbers.unwrap_or(true),
            },
            security: SecurityPreferences {
                remote_markdown_images: session
                    .remote_markdown_image_policy
                    .as_deref()
                    .and_then(RemoteMarkdownImagePolicy::from_key)
                    .unwrap_or_default(),
                check_for_updates_on_startup: session.check_for_updates_on_startup.unwrap_or(true),
            },
            merge_tool: MergeToolPreferences {
                auto_advance: session.mergetool_auto_advance.unwrap_or(true),
                collapse_unchanged: session.mergetool_collapse_unchanged.unwrap_or(false),
                output_scroll_sync: session.mergetool_output_scroll_sync.unwrap_or(true),
                show_line_numbers: session.mergetool_show_line_numbers.unwrap_or(true),
                view_three_way: session.mergetool_view_three_way.unwrap_or(true),
            },
            history: HistoryPreferences {
                show_graph: session.history_show_graph.unwrap_or(true),
                show_author: session.history_show_author.unwrap_or(true),
                show_date: session.history_show_date.unwrap_or(true),
                show_sha: session.history_show_sha.unwrap_or(false),
                relative_dates: session.history_relative_dates.unwrap_or(true),
                highlight_commit_chain: session.history_highlight_commit_chain.unwrap_or(true),
                show_tags: session.history_show_tags.unwrap_or(true),
                tag_fetch_mode: session.history_tag_fetch_mode.unwrap_or_default(),
                default_mode: session.default_history_mode.unwrap_or_default(),
            },
            file_editing: FileEditingPreferences {
                auto_save: session.auto_save_file_edits.unwrap_or(false),
            },
            repository: RepositoryPreferences {
                commit_push_after_enabled: session.commit_push_after_enabled.unwrap_or(false),
                default_tag_type: session.default_tag_type.unwrap_or_default(),
            },
            terminal: TerminalPreferences::from_ui_session(session),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_defaults_are_resolved_once() {
        let preferences = UiPreferences::from_session(&session::UiSession::default());
        assert_eq!(preferences, UiPreferences::default());
        assert_eq!(preferences.diff.view_mode, DiffViewMode::Split);
        assert!(preferences.diff.show_line_numbers);
        assert!(preferences.history.show_graph);
        assert!(preferences.merge_tool.view_three_way);
        assert_eq!(
            preferences.security.remote_markdown_images,
            RemoteMarkdownImagePolicy::AlwaysLoad
        );
        assert!(preferences.security.check_for_updates_on_startup);
    }

    #[test]
    fn security_preferences_are_parsed_from_session() {
        let preferences = UiPreferences::from_session(&session::UiSession {
            remote_markdown_image_policy: Some("ask".to_string()),
            check_for_updates_on_startup: Some(false),
            ..Default::default()
        });
        assert_eq!(
            preferences.security.remote_markdown_images,
            RemoteMarkdownImagePolicy::AskBeforeLoading
        );
        assert!(!preferences.security.check_for_updates_on_startup);
    }
}
