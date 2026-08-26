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

pub fn persist_ui_settings_to_path(settings: UiSettings, path: &Path) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        if settings.window_width.is_some() && settings.window_height.is_some() {
            file.window_width = settings.window_width;
            file.window_height = settings.window_height;
        }
        if let Some(w) = settings.sidebar_width {
            file.sidebar_width = Some(w);
        }
        if let Some(w) = settings.details_width {
            file.details_width = Some(w);
        }
        if let Some(collapsed) = settings.sidebar_collapsed {
            file.sidebar_collapsed = Some(collapsed);
        }
        if let Some(items) = settings.repo_sidebar_collapsed_items {
            let items = path_keyed_string_sets_to_storage(items);
            file.repo_sidebar_collapsed_items = (!items.is_empty()).then_some(items);
        }
        if let Some(items) = settings.repo_sidebar_pinned_branches {
            let items = path_keyed_string_sets_to_storage(items);
            file.repo_sidebar_pinned_branches = (!items.is_empty()).then_some(items);
        }
        if let Some(theme_mode) = settings.theme_mode {
            file.theme_mode = Some(theme_mode);
        }
        if let Some(percent) = settings.ui_scale_percent {
            file.ui_scale_percent = Some(percent);
        }
        if let Some(font_family) = settings.ui_font_family {
            file.ui_font_family = Some(font_family);
        }
        if let Some(font_family) = settings.editor_font_family {
            file.editor_font_family = Some(font_family);
        }
        if let Some(value) = settings.use_font_ligatures {
            file.use_font_ligatures = Some(value);
        }
        if let Some(fmt) = settings.date_time_format {
            file.date_time_format = Some(fmt);
        }
        if let Some(tz) = settings.timezone {
            file.timezone = Some(tz);
        }
        if let Some(value) = settings.show_timezone {
            file.show_timezone = Some(value);
        }
        if let Some(value) = settings.change_tracking_view {
            file.change_tracking_view = Some(value);
        }
        if let Some(value) = settings.repo_picker_sort {
            file.repo_picker_sort = Some(value);
        }
        // Owned by the repository picker (`repo_picker::persist_collapsed_sections`).
        if let Some(value) = settings.repo_picker_collapsed_sections {
            file.repo_picker_collapsed_sections = Some(value);
        }
        if let Some(value) = settings.diff_scroll_sync {
            file.diff_scroll_sync = Some(value);
        }
        if let Some(value) = settings.diff_content_mode {
            file.diff_content_mode = Some(value);
        }
        if let Some(value) = settings.diff_whitespace_mode {
            file.diff_whitespace_mode = Some(value);
        }
        if let Some(value) = settings.diff_view_mode {
            file.diff_view_mode = Some(value);
        }
        if let Some(value) = settings.annotate_enabled {
            file.annotate_enabled = Some(value);
        }
        if let Some(value) = settings.diff_reveal_whitespace_chars {
            file.diff_reveal_whitespace_chars = Some(value);
        }
        if let Some(value) = settings.mergetool_auto_advance {
            file.mergetool_auto_advance = Some(value);
        }
        if let Some(value) = settings.mergetool_collapse_unchanged {
            file.mergetool_collapse_unchanged = Some(value);
        }
        if let Some(value) = settings.mergetool_output_scroll_sync {
            file.mergetool_output_scroll_sync = Some(value);
        }
        if let Some(value) = settings.mergetool_show_line_numbers {
            file.mergetool_show_line_numbers = Some(value);
        }
        if let Some(value) = settings.mergetool_view_three_way {
            file.mergetool_view_three_way = Some(value);
        }
        if let Some(value) = settings.diff_word_wrap {
            file.diff_word_wrap = Some(value);
        }
        if let Some(value) = settings.auto_save_file_edits {
            file.auto_save_file_edits = Some(value);
        }
        if let Some(value) = settings.diff_show_line_numbers {
            file.diff_show_line_numbers = Some(value);
        }
        if let Some(value) = settings.change_tracking_height {
            file.change_tracking_height = Some(value);
        }
        if let Some(value) = settings.untracked_height {
            file.untracked_height = Some(value);
        }
        if let Some(value) = settings.history_show_graph {
            file.history_show_graph = Some(value);
        }
        if let Some(value) = settings.history_show_author {
            file.history_show_author = Some(value);
        }
        if let Some(value) = settings.history_show_date {
            file.history_show_date = Some(value);
        }
        if let Some(value) = settings.history_show_sha {
            file.history_show_sha = Some(value);
        }
        if let Some(value) = settings.terminal_external_mode {
            file.terminal_external_mode = Some(value);
        }
        if let Some(value) = settings.terminal_external_program {
            file.terminal_external_program = Some(value);
        }
        if let Some(value) = settings.terminal_external_args {
            let values = value
                .into_iter()
                .map(|arg| arg.trim().to_string())
                .filter(|arg| !arg.is_empty())
                .collect::<Vec<_>>();
            file.terminal_external_args = Some(values);
        }
        if let Some(value) = settings.terminal_action_bar_target {
            file.terminal_action_bar_target = Some(value);
        }
        if let Some(value) = settings.history_show_tags {
            file.history_show_tags = Some(value);
        }
        if let Some(value) = settings.history_highlight_commit_chain {
            file.history_highlight_commit_chain = Some(value);
        }
        if let Some(value) = settings.history_relative_dates {
            file.history_relative_dates = Some(value);
        }
        if let Some(value) = settings.history_tag_fetch_mode {
            file.history_tag_fetch_mode = Some(value);
        }
        if let Some(value) = settings.default_history_mode {
            file.default_history_mode = Some(value.into());
        }
        if let Some(value) = settings.commit_push_after_enabled {
            file.commit_push_after_enabled = Some(value);
        }
        if let Some(value) = settings.default_tag_type {
            file.default_tag_type = Some(value);
        }
        if let Some(path) = settings.git_executable_path {
            file.git_executable_path = path.map(|path| path_storage_key(&path));
        }
        if let Some(editor) = settings.external_code_editor {
            file.external_code_editor = editor.map(external_code_editor_to_file);
        }

        persist_to_path(path, &file)
    })
}
