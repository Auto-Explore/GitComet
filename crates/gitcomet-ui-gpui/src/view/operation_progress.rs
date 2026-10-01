//! The progress card for long git operations: fetch, pull and maintenance.
use gitcomet_core::git_operation::GitOperationId;
use gitcomet_core::git_progress::GitProgressMeter;
use gitcomet_state::model::{GitHookOperation, GitHookOperationStatus, RepoId};
use std::time::{Duration, SystemTime};

/// A card appears once git reports a meter, or once the operation has run
/// this long, so a quick fetch or maintenance never flashes one.
const SHOW_AFTER: Duration = Duration::from_secs(2);

/// Git recommends maintenance for a repository; its card asks the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MaintenanceRecommendation {
    pub(super) repo_id: RepoId,
    pub(super) repo_name: String,
}

pub(super) const MAINTENANCE_RECOMMENDATION_TEXT: &str = "Git recommends running maintenance on \
     this repository. It repacks objects so Git stays fast; large repositories can take several \
     minutes.";

/// What the card shows of one running operation; compared on every state
/// apply, so it leaves out the operation's output buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OperationProgress {
    pub(super) repo_id: RepoId,
    pub(super) operation_id: GitOperationId,
    pub(super) label: String,
    /// Repository name and the operation's context, e.g. `app · All remotes`.
    pub(super) subtitle: String,
    pub(super) started: SystemTime,
    pub(super) cancelling: bool,
    pub(super) progress: Option<GitProgressMeter>,
}

impl OperationProgress {
    pub(super) fn from_operation(
        repo_id: RepoId,
        repo_name: &str,
        operation: &GitHookOperation,
    ) -> Self {
        let subtitle = match operation.context.as_deref() {
            Some(context) if !context.is_empty() => format!("{repo_name} · {context}"),
            _ => repo_name.to_string(),
        };
        Self {
            repo_id,
            operation_id: operation.id,
            label: operation.label.clone(),
            subtitle,
            started: operation.time,
            cancelling: operation.status == GitHookOperationStatus::Cancelling,
            progress: operation.progress.clone(),
        }
    }

    /// GitComet's own estimates (maintenance) start at once, so only git's
    /// meters skip the wait.
    pub(super) fn is_visible(&self, now: SystemTime) -> bool {
        self.progress
            .as_ref()
            .is_some_and(|progress| !progress.estimated)
            || now
                .duration_since(self.started)
                .is_ok_and(|elapsed| elapsed >= SHOW_AFTER)
    }

    pub(super) fn title(&self) -> String {
        if self.cancelling {
            return "Stopping…".to_string();
        }
        match self.label.as_str() {
            "Fetch" => "Fetching…".to_string(),
            "Pull" => "Pulling…".to_string(),
            "Prune branches" => "Fetching and pruning branches…".to_string(),
            "Maintenance" => "Optimizing repository…".to_string(),
            label => format!("{label}…"),
        }
    }

    /// What the operation is for, where its title says too little.
    pub(super) fn explanation(&self) -> Option<&'static str> {
        (self.label == "Maintenance").then_some(
            "Git is repacking objects so it stays fast. Large repositories can take several \
             minutes.",
        )
    }

    pub(super) fn phase(&self) -> &str {
        self.progress
            .as_ref()
            .map_or("Starting…", |progress| &progress.title)
    }

    /// `46%`, or `~46%` for a GitComet estimate.
    pub(super) fn percent_label(&self) -> Option<String> {
        let progress = self.progress.as_ref()?;
        let percent = progress.percent?;
        Some(if progress.estimated {
            format!("~{percent}%")
        } else {
            format!("{percent}%")
        })
    }

    pub(super) fn elapsed(&self, now: SystemTime) -> String {
        format_elapsed(now.duration_since(self.started).unwrap_or_default())
    }
}

pub(super) fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn progress(label: &str, meter: Option<GitProgressMeter>) -> OperationProgress {
        OperationProgress {
            repo_id: RepoId(1),
            operation_id: GitOperationId(1),
            label: label.to_string(),
            subtitle: "app".to_string(),
            started: SystemTime::UNIX_EPOCH,
            cancelling: false,
            progress: meter,
        }
    }

    #[test]
    fn elapsed_reads_as_seconds_minutes_or_hours() {
        assert_eq!(format_elapsed(Duration::from_secs(7)), "7s");
        assert_eq!(format_elapsed(Duration::from_secs(133)), "2m 13s");
        assert_eq!(format_elapsed(Duration::from_secs(3840)), "1h 04m");
    }

    #[test]
    fn card_waits_for_a_meter_or_two_seconds() {
        let started = SystemTime::UNIX_EPOCH;
        let mut quiet = progress("Fetch", None);
        assert!(!quiet.is_visible(started + Duration::from_secs(1)));
        assert!(quiet.is_visible(started + SHOW_AFTER));
        quiet.progress = Some(GitProgressMeter {
            title: Arc::from("Receiving objects"),
            percent: Some(3),
            estimated: false,
        });
        assert!(quiet.is_visible(started));
    }

    #[test]
    fn estimated_meter_waits_like_a_quiet_operation() {
        // Maintenance's poller reports at once; a run done within a second
        // shows no card, only its result.
        let started = SystemTime::UNIX_EPOCH;
        let maintenance = progress(
            "Maintenance",
            Some(GitProgressMeter::estimated("Writing new pack", 10)),
        );
        assert!(!maintenance.is_visible(started + Duration::from_secs(1)));
        assert!(maintenance.is_visible(started + SHOW_AFTER));
    }

    #[test]
    fn estimates_are_marked_and_titles_follow_the_operation() {
        let measured = progress("Fetch", Some(GitProgressMeter::estimated("Writing", 46)));
        assert_eq!(measured.percent_label().as_deref(), Some("~46%"));
        assert_eq!(measured.title(), "Fetching…");
        assert_eq!(progress("Pull", None).phase(), "Starting…");
        assert_eq!(progress("Pull", None).percent_label(), None);
        let mut stopping = progress("Pull", None);
        stopping.cancelling = true;
        assert_eq!(stopping.title(), "Stopping…");
    }
}
