use super::*;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UiSettings {
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub sidebar_width: Option<u32>,
    pub details_width: Option<u32>,
    pub sidebar_collapsed: Option<bool>,
    pub repo_sidebar_collapsed_items: Option<BTreeMap<PathBuf, BTreeSet<String>>>,
    pub repo_sidebar_pinned_branches: Option<BTreeMap<PathBuf, BTreeSet<String>>>,
    pub theme_mode: Option<String>,
    pub ui_scale_percent: Option<u32>,
    pub ui_font_family: Option<String>,
    pub editor_font_family: Option<String>,
    pub use_font_ligatures: Option<bool>,
    pub date_time_format: Option<String>,
    pub timezone: Option<String>,
    pub show_timezone: Option<bool>,
    pub change_tracking_view: Option<String>,
    pub repo_picker_sort: Option<String>,
    /// Whole replacement set — the repository picker owns it and always writes
    /// every collapsed section it knows about.
    pub repo_picker_collapsed_sections: Option<BTreeSet<String>>,
    pub diff_scroll_sync: Option<String>,
    pub diff_content_mode: Option<String>,
    pub diff_whitespace_mode: Option<String>,
    pub diff_view_mode: Option<String>,
    pub annotate_enabled: Option<bool>,
    pub diff_reveal_whitespace_chars: Option<bool>,
    pub diff_word_wrap: Option<bool>,
    pub diff_show_line_numbers: Option<bool>,
    pub remote_markdown_image_policy: Option<String>,
    pub allowed_remote_protocols: Option<BTreeSet<String>>,
    pub check_for_updates_on_startup: Option<bool>,
    pub auto_save_file_edits: Option<bool>,
    pub mergetool_auto_advance: Option<bool>,
    pub mergetool_collapse_unchanged: Option<bool>,
    pub mergetool_output_scroll_sync: Option<bool>,
    pub mergetool_show_line_numbers: Option<bool>,
    pub mergetool_view_three_way: Option<bool>,
    pub change_tracking_height: Option<u32>,
    pub untracked_height: Option<u32>,
    pub history_show_graph: Option<bool>,
    pub history_show_author: Option<bool>,
    pub history_show_date: Option<bool>,
    pub history_show_sha: Option<bool>,
    pub terminal_external_mode: Option<String>,
    pub terminal_external_program: Option<String>,
    pub terminal_external_args: Option<Vec<String>>,
    pub terminal_action_bar_target: Option<String>,
    pub history_show_tags: Option<bool>,
    pub history_relative_dates: Option<bool>,
    pub history_highlight_commit_chain: Option<bool>,
    pub history_tag_fetch_mode: Option<GitLogTagFetchMode>,
    pub default_history_mode: Option<HistoryMode>,
    pub commit_push_after_enabled: Option<bool>,
    pub default_tag_type: Option<DefaultTagType>,
    pub git_executable_path: Option<Option<PathBuf>>,
    pub external_code_editor: Option<Option<ExternalCodeEditorSetting>>,
}

pub fn persist_ui_settings(settings: UiSettings) -> io::Result<()> {
    let Some(path) = default_session_file_path() else {
        return Ok(());
    };
    persist_ui_settings_to_path(settings, &path)
}

/// Copies one optional `UiSettings` field onto the session file when set.
macro_rules! apply_setting {
    ($settings:expr, $file:expr, $field:ident) => {
        if let Some(value) = $settings.$field {
            $file.$field = Some(value);
        }
    };
}

pub fn persist_ui_settings_to_path(settings: UiSettings, path: &Path) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        if settings.window_width.is_some() && settings.window_height.is_some() {
            file.window_width = settings.window_width;
            file.window_height = settings.window_height;
        }
        apply_setting!(settings, file, sidebar_width);
        apply_setting!(settings, file, details_width);
        apply_setting!(settings, file, sidebar_collapsed);
        if let Some(items) = settings.repo_sidebar_collapsed_items {
            let items = path_keyed_string_sets_to_storage(items);
            file.repo_sidebar_collapsed_items = (!items.is_empty()).then_some(items);
        }
        if let Some(items) = settings.repo_sidebar_pinned_branches {
            let items = path_keyed_string_sets_to_storage(items);
            file.repo_sidebar_pinned_branches = (!items.is_empty()).then_some(items);
        }
        apply_setting!(settings, file, theme_mode);
        apply_setting!(settings, file, ui_scale_percent);
        apply_setting!(settings, file, ui_font_family);
        apply_setting!(settings, file, editor_font_family);
        apply_setting!(settings, file, use_font_ligatures);
        apply_setting!(settings, file, date_time_format);
        apply_setting!(settings, file, timezone);
        apply_setting!(settings, file, show_timezone);
        apply_setting!(settings, file, change_tracking_view);
        apply_setting!(settings, file, repo_picker_sort);
        // Owned by the repository picker (`repo_picker::persist_collapsed_sections`).
        apply_setting!(settings, file, repo_picker_collapsed_sections);
        apply_setting!(settings, file, diff_scroll_sync);
        apply_setting!(settings, file, diff_content_mode);
        apply_setting!(settings, file, diff_whitespace_mode);
        apply_setting!(settings, file, diff_view_mode);
        apply_setting!(settings, file, annotate_enabled);
        apply_setting!(settings, file, diff_reveal_whitespace_chars);
        apply_setting!(settings, file, mergetool_auto_advance);
        apply_setting!(settings, file, mergetool_collapse_unchanged);
        apply_setting!(settings, file, mergetool_output_scroll_sync);
        apply_setting!(settings, file, mergetool_show_line_numbers);
        apply_setting!(settings, file, mergetool_view_three_way);
        apply_setting!(settings, file, diff_word_wrap);
        apply_setting!(settings, file, remote_markdown_image_policy);
        apply_setting!(settings, file, allowed_remote_protocols);
        apply_setting!(settings, file, check_for_updates_on_startup);
        apply_setting!(settings, file, auto_save_file_edits);
        apply_setting!(settings, file, diff_show_line_numbers);
        apply_setting!(settings, file, change_tracking_height);
        apply_setting!(settings, file, untracked_height);
        apply_setting!(settings, file, history_show_graph);
        apply_setting!(settings, file, history_show_author);
        apply_setting!(settings, file, history_show_date);
        apply_setting!(settings, file, history_show_sha);
        apply_setting!(settings, file, terminal_external_mode);
        apply_setting!(settings, file, terminal_external_program);
        if let Some(value) = settings.terminal_external_args {
            let values = value
                .into_iter()
                .map(|arg| arg.trim().to_string())
                .filter(|arg| !arg.is_empty())
                .collect::<Vec<_>>();
            file.terminal_external_args = Some(values);
        }
        apply_setting!(settings, file, terminal_action_bar_target);
        apply_setting!(settings, file, history_show_tags);
        apply_setting!(settings, file, history_highlight_commit_chain);
        apply_setting!(settings, file, history_relative_dates);
        apply_setting!(settings, file, history_tag_fetch_mode);
        if let Some(value) = settings.default_history_mode {
            file.default_history_mode = Some(value.into());
        }
        apply_setting!(settings, file, commit_push_after_enabled);
        apply_setting!(settings, file, default_tag_type);
        if let Some(path) = settings.git_executable_path {
            file.git_executable_path = path.map(|path| path_storage_key(&path));
        }
        if let Some(editor) = settings.external_code_editor {
            file.external_code_editor = editor.map(external_code_editor_to_file);
        }

        persist_to_path(path, &file)
    })
}
