//! The status strip under file views: how the shown file was read, and the
//! user's encoding / line-ending choices for it.

use super::*;
use gitcomet_core::domain::FileDiffTextSource;
use gitcomet_core::text_format::{
    EolSource, FormatSource, LineEnding, LineEndingStats, SideTextFormat, TabWidthSource,
    TextAttributes, TextEncoding, TextOverride,
};
use std::path::Path;

/// What the strip shows for the file on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::view) struct TextFormatStatus {
    pub(in crate::view) encoding_label: SharedString,
    pub(in crate::view) encoding_tooltip: SharedString,
    /// Invalid bytes, a lossy encoding or a low-confidence guess.
    pub(in crate::view) encoding_warning: bool,
    pub(in crate::view) line_ending_label: Option<SharedString>,
    pub(in crate::view) line_ending_tooltip: SharedString,
    /// The view writes the file, so line endings can be converted.
    pub(in crate::view) editable: bool,
    pub(in crate::view) tab_label: SharedString,
    pub(in crate::view) tab_tooltip: SharedString,
}

/// What the encoding menu needs.
#[derive(Clone, Debug)]
pub(in crate::view) struct TextEncodingMenuState {
    pub(in crate::view) path: std::path::PathBuf,
    /// The encoding the user chose, if any.
    pub(in crate::view) chosen: Option<TextEncoding>,
    /// What the file reads as now.
    pub(in crate::view) current: Option<TextEncoding>,
    /// `working-tree-encoding` applies: git stores the file as UTF-8.
    pub(in crate::view) stored_as_utf8: bool,
    pub(in crate::view) had_bom: bool,
    /// The editor shows the file, so it can be saved in another encoding.
    pub(in crate::view) editor: bool,
}

fn source_description(source: FormatSource, attributes: &TextAttributes) -> String {
    let label = |attr: &Option<gitcomet_core::text_format::EncodingAttr>| {
        attr.as_ref()
            .map(|attr| attr.label.to_string())
            .unwrap_or_default()
    };
    match source {
        FormatSource::Override => "Chosen for this file".to_string(),
        FormatSource::WorkingTreeEncoding => format!(
            "working-tree-encoding={} in .gitattributes",
            label(&attributes.working_tree_encoding)
        ),
        FormatSource::GitInternalUtf8 => {
            "Git stores this file as UTF-8 (working-tree-encoding)".to_string()
        }
        FormatSource::Bom => "Byte order mark".to_string(),
        FormatSource::EncodingAttribute => {
            format!("encoding={} in .gitattributes", label(&attributes.encoding))
        }
        FormatSource::Utf8 => "Valid UTF-8".to_string(),
        FormatSource::GuiEncoding => {
            format!(
                "gui.encoding={} in git config",
                label(&attributes.gui_encoding)
            )
        }
        FormatSource::Detected { confident: true } => "Detected".to_string(),
        FormatSource::Detected { confident: false } => {
            "Guessed; pick the right encoding if the text looks wrong".to_string()
        }
        FormatSource::Binary => "Binary".to_string(),
    }
}

fn encoding_name(format: &SideTextFormat) -> String {
    let mut name = format.format.encoding.name().to_string();
    if format.format.bom && format.format.encoding.is_utf8() {
        name.push_str(" with BOM");
    }
    name
}

fn line_ending_name(stats: LineEndingStats) -> Option<&'static str> {
    if stats.total() == 0 {
        None
    } else if stats.is_mixed() {
        Some("Mixed")
    } else {
        stats.dominant().map(LineEnding::label)
    }
}

fn eol_policy_description(attributes: &TextAttributes) -> String {
    let policy = attributes.eol_policy;
    let why = match policy.source {
        EolSource::None => return "Git keeps line endings as they are".to_string(),
        EolSource::EolAttribute => "eol in .gitattributes",
        EolSource::TextAttribute => "text in .gitattributes with core.eol / core.autocrlf",
        EolSource::CoreAutocrlf => "core.autocrlf",
    };
    let checkout = policy.checkout.map_or("as stored", LineEnding::label);
    format!("Git stores LF and checks out {checkout} ({why})")
}

fn format_details(format: &SideTextFormat, attributes: &TextAttributes) -> String {
    let mut details = vec![source_description(format.source, attributes)];
    if format.malformed {
        details.push("Some bytes are not valid in this encoding and show as �".to_string());
    }
    if format.lossy {
        details.push("Saving in this encoding would change bytes; editing is off".to_string());
    }
    details.join("\n")
}

impl MainPaneView {
    /// Saving can be called from a root-view close handler, so show errors
    /// after that handler releases its borrow of the root.
    pub(super) fn show_text_format_error(
        &self,
        message: impl Into<String>,
        cx: &mut gpui::Context<Self>,
    ) {
        let root = self.root_view.clone();
        let message = message.into();
        cx.defer(move |cx| {
            let _ = root.update(cx, |root, cx| {
                root.push_toast(crate::view::components::ToastKind::Error, message, cx);
            });
        });
    }

    /// How the conflicted file was read from the working tree.
    pub(in crate::view) fn conflict_current_text_format(&self) -> Option<SideTextFormat> {
        self.active_repo()?
            .conflict_state
            .conflict_session
            .as_ref()?
            .current_format
    }

    /// The resolved output as bytes in the conflicted file's encoding, or
    /// `None` after telling the user why it cannot be written.
    pub(in crate::view) fn conflict_output_bytes_for_save(
        &mut self,
        text: String,
        cx: &mut gpui::Context<Self>,
    ) -> Option<gitcomet_state::msg::ContentBytes> {
        match super::file_editor::encode_for_save(text.into(), self.conflict_current_text_format())
        {
            Ok((bytes, _)) => Some(bytes),
            Err(message) => {
                self.show_text_format_error(message, cx);
                None
            }
        }
    }

    fn selected_text_attributes(&self) -> Arc<TextAttributes> {
        match self
            .active_repo()
            .map(|repo| &repo.diff_state.text_attributes)
        {
            Some(Loadable::Ready(attributes)) => Arc::clone(attributes),
            _ => Arc::default(),
        }
    }

    /// Formats of what the pane shows: `(old, new, editable)`, one side for
    /// a whole-file view.
    fn shown_text_formats(&self) -> Option<(Option<SideTextFormat>, SideTextFormat, bool)> {
        match self.main_pane_surface().body {
            MainPaneBody::FileEditor => Some((None, self.file_editor_text_format?, true)),
            MainPaneBody::FilePreview => Some((None, self.worktree_preview_text_format?, false)),
            MainPaneBody::Conflict => Some((None, self.conflict_current_text_format()?, false)),
            MainPaneBody::FileDiff | MainPaneBody::Patch => {
                let repo = self.active_repo()?;
                let Loadable::Ready(Some(file)) = &repo.diff_state.diff_file else {
                    return None;
                };
                let side =
                    |source: &Option<FileDiffTextSource>| source.as_ref().and_then(|s| s.format);
                match (side(&file.old_source), side(&file.new_source)) {
                    (old, Some(new)) => Some((old, new, false)),
                    (Some(old), None) => Some((None, old, false)),
                    (None, None) => None,
                }
            }
            _ => None,
        }
    }

    pub(in crate::view) fn text_format_status(&self) -> Option<TextFormatStatus> {
        let (old, new, editable) = self.shown_text_formats()?;
        let attributes = self.selected_text_attributes();
        let differs = old.is_some_and(|old| old.format.encoding != new.format.encoding);
        let (encoding_label, encoding_tooltip) = match old.filter(|_| differs) {
            Some(old) => (
                format!("{} → {}", encoding_name(&old), encoding_name(&new)),
                format!(
                    "Old: {}\nNew: {}",
                    format_details(&old, &attributes),
                    format_details(&new, &attributes)
                ),
            ),
            None => (encoding_name(&new), format_details(&new, &attributes)),
        };
        let warn = |format: &SideTextFormat| {
            format.malformed
                || format.lossy
                || matches!(format.source, FormatSource::Detected { confident: false })
        };
        let encoding_warning = warn(&new) || old.as_ref().is_some_and(warn);

        let old_eol = old.and_then(|old| line_ending_name(old.line_endings));
        let new_eol = line_ending_name(new.line_endings);
        let line_ending_label = match (old_eol, new_eol) {
            (Some(old), Some(new)) if old != new => Some(format!("{old} → {new}")),
            (_, Some(new)) => Some(new.to_string()),
            (Some(old), None) => Some(old.to_string()),
            (None, None) => None,
        };
        let mut line_ending_tooltip = eol_policy_description(&attributes);
        if new.line_endings.is_mixed() {
            let stats = new.line_endings;
            line_ending_tooltip.push_str(&format!(
                "\n{} LF, {} CRLF, {} CR",
                stats.lf, stats.crlf, stats.cr
            ));
        }
        let (tab_size, tab_source) = self.effective_tab_size();
        Some(TextFormatStatus {
            encoding_label: encoding_label.into(),
            encoding_tooltip: encoding_tooltip.into(),
            encoding_warning,
            line_ending_label: line_ending_label.map(Into::into),
            line_ending_tooltip: line_ending_tooltip.into(),
            editable,
            tab_label: format!("Tab: {tab_size}").into(),
            tab_tooltip: tab_source.into(),
        })
    }

    /// The tab width for the file shown, and where it came from: the user's
    /// choice, then `.gitattributes` / `core.whitespace`, then Settings.
    pub(in crate::view) fn effective_tab_size(&self) -> (u8, &'static str) {
        let max = crate::view::tab_width::MAX_TAB_WIDTH;
        if let Some(repo) = self.active_repo()
            && let Some(path) = repo
                .diff_state
                .diff_target
                .as_ref()
                .and_then(DiffTarget::file_path)
        {
            if let Some(size) = repo
                .diff_state
                .text_override_for(path)
                .and_then(|value| value.tab_size)
            {
                return (size.clamp(1, max), "Chosen for this file");
            }
            if let Loadable::Ready(attributes) = &repo.diff_state.text_attributes
                && let Some(width) = attributes.tab_width
            {
                let source = match width.source {
                    TabWidthSource::Attribute => "whitespace=tabwidth in .gitattributes",
                    TabWidthSource::CoreWhitespace => "core.whitespace tabwidth in git config",
                };
                return (width.columns.clamp(1, max), source);
            }
        }
        (self.default_tab_size.clamp(1, max), "Settings")
    }

    pub(in crate::view) fn set_default_tab_size(&mut self, size: u8, cx: &mut gpui::Context<Self>) {
        self.default_tab_size = size;
        cx.notify();
    }

    /// Point the text views at the shown file's tab width, dropping what was
    /// rendered with another. Called before rows are built each frame.
    pub(in crate::view) fn sync_display_tab_width(&mut self, cx: &mut gpui::Context<Self>) {
        let (size, _) = self.effective_tab_size();
        // The editors keep tabs as characters and widen them when shaping.
        for input in [&self.file_editor_input, &self.conflict_resolver_input] {
            input.update(cx, |input, cx| input.set_tab_size(usize::from(size), cx));
        }
        let width = usize::from(size);
        if self.display_tab_width == width {
            return;
        }
        self.display_tab_width = width;
        self.clear_diff_text_style_caches();
        self.clear_diff_text_projected_highlights();
        self.clear_conflict_diff_style_caches();
        self.clear_conflict_diff_query_overlay_caches();
        self.conflict_three_way_segments_cache.clear();
        self.conflict_three_way_query_segments_cache.clear();
        self.clear_worktree_preview_segments_cache();
        self.worktree_preview_style_cache_epoch =
            self.worktree_preview_style_cache_epoch.wrapping_add(1);
        self.diff_wrap_visible_cache_key = None;
        self.diff_wrap_visible_rows = Arc::from([]);
        self.diff_scrollbar_markers_cache.clear();
        self.diff_search_inline_patch_trigram_index = None;
        if self.diff_search_active && !self.diff_search_query.is_empty() {
            self.diff_search_recompute_matches_preserving_current();
        }
    }

    /// Use `size` for the open file's tabs; `None` goes back to its
    /// attributes or Settings.
    pub(in crate::view) fn set_tab_size_override(&mut self, size: Option<u8>) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let repo_id = repo.id;
        let Some(path) = repo
            .diff_state
            .diff_target
            .as_ref()
            .and_then(DiffTarget::file_path)
            .map(Path::to_path_buf)
        else {
            return;
        };
        let value = TextOverride {
            tab_size: size,
            ..repo.diff_state.text_override_for(&path).unwrap_or_default()
        };
        self.store.dispatch(Msg::SetTextOverride {
            repo_id,
            path,
            value,
        });
    }

    pub(in crate::view) fn text_encoding_menu_state(&self) -> Option<TextEncodingMenuState> {
        let repo = self.active_repo()?;
        let path = repo
            .diff_state
            .diff_target
            .as_ref()?
            .file_path()?
            .to_path_buf();
        let (_, shown, editable) = self.shown_text_formats()?;
        let attributes = self.selected_text_attributes();
        Some(TextEncodingMenuState {
            editor: editable && !shown.malformed && !shown.binary,
            chosen: repo
                .diff_state
                .text_override_for(&path)
                .and_then(|value| value.encoding),
            current: Some(shown.format.encoding),
            stored_as_utf8: attributes.working_tree_encoding().is_some(),
            had_bom: shown.format.bom,
            path,
        })
    }

    /// Read the open file in `encoding`, or as its attributes and content say
    /// when `None`. An editor with unsaved edits keeps them and refuses.
    pub(in crate::view) fn set_text_encoding_override(
        &mut self,
        encoding: Option<TextEncoding>,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let repo_id = repo.id;
        let Some(path) = repo
            .diff_state
            .diff_target
            .as_ref()
            .and_then(DiffTarget::file_path)
            .map(Path::to_path_buf)
        else {
            return;
        };
        let unsaved_editor =
            self.main_pane_surface().body == MainPaneBody::FileEditor && self.file_editor_dirty;
        let unsaved_resolution = self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
            && self.conflict_resolved_output_is_modified();
        if unsaved_editor || unsaved_resolution {
            self.show_text_format_error(
                "Save or discard your edits before reopening the file in another encoding",
                cx,
            );
            return;
        }
        let value = TextOverride {
            encoding,
            ..repo.diff_state.text_override_for(&path).unwrap_or_default()
        };
        self.store.dispatch(Msg::SetTextOverride {
            repo_id,
            path,
            value,
        });
    }

    /// Save the editor's buffer as `format` from now on — a conversion, so the
    /// buffer counts as unsaved until it is written.
    pub(in crate::view) fn set_file_editor_save_format(
        &mut self,
        format: gitcomet_core::text_format::TextFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.main_pane_surface().body != MainPaneBody::FileEditor || self.file_editor_loading {
            return;
        }
        let Some(mut current) = self.file_editor_text_format else {
            return;
        };
        // Text with replacement characters would write them into the file.
        if current.malformed || current.binary || current.format == format {
            return;
        }
        current.format = format;
        current.source = FormatSource::Override;
        // The old encoding's round-trip worry does not apply to the new one.
        current.lossy = false;
        self.set_file_editor_text_format(Some(current), cx);
        self.file_editor_dirty = true;
        self.file_editor_saved_fingerprint = None;
        cx.notify();
    }

    /// Rewrite every line break in the editor as `ending`, as one undoable
    /// edit, and use it for new lines.
    pub(in crate::view) fn convert_file_editor_line_endings(
        &mut self,
        ending: LineEnding,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.main_pane_surface().body != MainPaneBody::FileEditor || self.file_editor_loading {
            return;
        }
        let converted = self.file_editor_input.update(cx, |input, cx| {
            input.set_line_ending(ending.as_str());
            let text = input.text();
            let converted = gitcomet_core::text_format::convert_line_endings(text, ending);
            let converted = match converted {
                std::borrow::Cow::Borrowed(_) => return None,
                std::borrow::Cow::Owned(converted) => converted,
            };
            let len = text.len();
            input.replace_utf8_range(0..len, &converted, cx);
            Some(LineEndingStats::from_bytes(converted.as_bytes()))
        });
        if let (Some(stats), Some(format)) = (converted, self.file_editor_text_format.as_mut()) {
            format.line_endings = stats;
        }
        cx.notify();
    }

    /// Add an attribute for the open file to the repository's `.gitattributes`.
    pub(in crate::view) fn add_text_encoding_rule(&mut self, rule: String) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        self.store
            .dispatch(Msg::AppendGitattributesRule { repo_id, rule });
    }
}
