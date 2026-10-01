//! Errors shown as toasts that stay until the user closes them, and what the
//! details dialog shows and offers for each.

use super::*;
use gitcomet_core::text_format::TextFormat;
use std::path::PathBuf;
use std::time::SystemTime;

/// An error to show the user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ErrorReport {
    pub(crate) repo_id: Option<RepoId>,
    pub(crate) message: String,
    pub(crate) actions: Vec<ErrorAction>,
}

impl ErrorReport {
    pub(crate) fn message(repo_id: Option<RepoId>, message: impl Into<String>) -> Self {
        Self {
            repo_id,
            message: message.into(),
            actions: Vec::new(),
        }
    }

    pub(crate) fn with_action(mut self, action: ErrorAction) -> Self {
        self.actions.push(action);
        self
    }
}

/// Something the details dialog can do about an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ErrorAction {
    /// Write the editor's buffer of `path` in `format`.
    SaveEditorAs {
        repo_id: RepoId,
        path: PathBuf,
        format: TextFormat,
    },
    /// Put the editor's caret on `ch`, at a 1-based line and column.
    RevealInEditor {
        repo_id: RepoId,
        path: PathBuf,
        line: u32,
        column: u32,
        ch: char,
    },
    OpenUrl {
        url: String,
        label: String,
    },
}

impl ErrorAction {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::SaveEditorAs { format, .. } => {
                let bom = if format.bom && !format.encoding.is_utf16() {
                    " with BOM"
                } else {
                    ""
                };
                format!("Save as {}{bom}", format.encoding.name())
            }
            Self::RevealInEditor { ch, .. } => format!("Go to ‘{ch}’"),
            Self::OpenUrl { label, .. } => label.clone(),
        }
    }
}

/// An error on screen: its text, its actions, and how often it has happened.
#[derive(Clone, Debug)]
pub(crate) struct ErrorNotice {
    pub(crate) repo_id: Option<RepoId>,
    /// As reported; identical messages share one notice.
    pub(crate) message: String,
    pub(crate) text: ErrorText,
    pub(crate) actions: Vec<ErrorAction>,
    /// When it last happened.
    pub(crate) time: SystemTime,
    pub(crate) count: u32,
}

impl ErrorNotice {
    pub(crate) fn new(report: ErrorReport) -> Self {
        Self {
            text: ErrorText::parse(&report.message),
            repo_id: report.repo_id,
            message: report.message,
            actions: report.actions,
            time: SystemTime::now(),
            count: 1,
        }
    }

    /// The same error again.
    pub(crate) fn repeat(&mut self, report: ErrorReport) {
        self.count = self.count.saturating_add(1);
        self.time = SystemTime::now();
        self.repo_id = self.repo_id.or(report.repo_id);
        if !report.actions.is_empty() {
            self.actions = report.actions;
        }
    }

    /// Everything the dialog shows, for the clipboard.
    pub(crate) fn details_for_copy(&self) -> String {
        let mut out = self.text.summary.clone();
        if let Some(command) = &self.text.command {
            out.push_str("\n\n");
            out.push_str(command);
        }
        if let Some(details) = &self.text.details {
            out.push_str("\n\n");
            out.push_str(details);
        }
        out
    }
}

/// An error message split the way `format_failure_summary` writes one: a
/// first paragraph, then 4-space-indented blocks with the `git` command and
/// its output.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ErrorText {
    /// The first paragraph, e.g. "Push failed".
    pub(crate) summary: String,
    /// The first indented block that is a `git` command, unindented.
    pub(crate) command: Option<String>,
    /// The rest, unindented, with runs of blank lines collapsed.
    pub(crate) details: Option<String>,
}

impl ErrorText {
    pub(crate) fn parse(message: &str) -> Self {
        let lines = message.trim_matches('\n').lines().collect::<Vec<_>>();
        let is_code = |line: &str| line.starts_with("    ");
        let summary_end = lines
            .iter()
            .position(|line| line.trim().is_empty() || is_code(line))
            .unwrap_or(lines.len());
        let summary = lines[..summary_end]
            .iter()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join(" ");
        let summary = summary.trim_end_matches(':').trim_end().to_string();

        let mut command = None;
        let mut rest = Vec::new();
        let mut ix = summary_end;
        while ix < lines.len() {
            let line = lines[ix];
            let starts_command = command.is_none()
                && is_code(line)
                && line.trim_start().starts_with("git ")
                && (ix == 0 || !is_code(lines[ix - 1]));
            if starts_command {
                let start = ix;
                while ix < lines.len() && is_code(lines[ix]) {
                    ix += 1;
                }
                command = Some(
                    lines[start..ix]
                        .iter()
                        .map(|line| &line[4..])
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                continue;
            }
            rest.push(line.strip_prefix("    ").unwrap_or(line));
            ix += 1;
        }

        let mut details = String::new();
        let mut blank_run = false;
        for line in rest {
            let blank = line.trim().is_empty();
            if blank && (blank_run || details.is_empty()) {
                blank_run = true;
                continue;
            }
            if !details.is_empty() {
                details.push('\n');
            }
            details.push_str(line);
            blank_run = blank;
        }
        let details = details.trim_end().to_string();

        if summary.is_empty() {
            // A message that starts with output: lead with its first line.
            let mut body = details.lines();
            let first = body.next().unwrap_or_default().trim().to_string();
            let remainder = body.collect::<Vec<_>>().join("\n").trim().to_string();
            return Self {
                summary: first,
                command,
                details: (!remainder.is_empty()).then_some(remainder),
            };
        }
        Self {
            summary,
            command,
            details: (!details.is_empty()).then_some(details),
        }
    }

    /// One line under the summary in the toast: where the output starts.
    pub(crate) fn preview(&self) -> Option<&str> {
        self.details
            .as_deref()
            .and_then(|details| details.lines().find(|line| !line.trim().is_empty()))
            .map(str::trim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_git_command_splits_into_summary_command_and_output() {
        let text = ErrorText::parse(
            "Push failed:\n\n    git push origin main\n\n     ! [rejected] main -> main (fetch first)\n    error: failed to push some refs",
        );
        assert_eq!(text.summary, "Push failed");
        assert_eq!(text.command.as_deref(), Some("git push origin main"));
        assert_eq!(
            text.details.as_deref(),
            Some(" ! [rejected] main -> main (fetch first)\nerror: failed to push some refs")
        );
        assert_eq!(
            text.preview(),
            Some("! [rejected] main -> main (fetch first)")
        );
    }

    #[test]
    fn a_one_line_message_is_all_summary() {
        let text = ErrorText::parse("Cannot push: no remotes configured");
        assert_eq!(
            text,
            ErrorText {
                summary: "Cannot push: no remotes configured".into(),
                command: None,
                details: None,
            }
        );
        assert_eq!(text.preview(), None);
    }

    #[test]
    fn prose_details_and_later_code_blocks_are_kept_in_order() {
        let text = ErrorText::parse(
            "Pull failed:\n\nNot possible to fast-forward.\n\n\n    hint: use --rebase\n    hint: or --no-ff",
        );
        assert_eq!(text.summary, "Pull failed");
        assert_eq!(text.command, None);
        assert_eq!(
            text.details.as_deref(),
            Some("Not possible to fast-forward.\n\nhint: use --rebase\nhint: or --no-ff")
        );
    }

    #[test]
    fn a_message_that_starts_with_output_leads_with_its_first_line() {
        let text = ErrorText::parse("    fatal: not a git repository\n    (or any parent)");
        assert_eq!(text.summary, "fatal: not a git repository");
        assert_eq!(text.details.as_deref(), Some("(or any parent)"));
    }

    #[test]
    fn repeats_count_and_keep_the_first_repo() {
        let mut notice = ErrorNotice::new(ErrorReport::message(None, "boom"));
        notice.repeat(ErrorReport::message(Some(RepoId(3)), "boom"));
        notice.repeat(ErrorReport::message(Some(RepoId(4)), "boom"));
        assert_eq!(notice.count, 3);
        assert_eq!(notice.repo_id, Some(RepoId(3)));
    }

    #[test]
    fn save_labels_name_the_encoding() {
        let save = |format| ErrorAction::SaveEditorAs {
            repo_id: RepoId(1),
            path: "a.txt".into(),
            format,
        };
        assert_eq!(save(TextFormat::UTF_8).label(), "Save as UTF-8");
        assert_eq!(
            save(TextFormat {
                encoding: gitcomet_core::text_format::TextEncoding::UTF_8,
                bom: true,
            })
            .label(),
            "Save as UTF-8 with BOM"
        );
    }
}
