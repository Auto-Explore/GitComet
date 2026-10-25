use super::*;
use gitcomet_core::process::{GitRuntimeState, GitVersion};

const GIT_DOWNLOAD_URL: &str = "https://git-scm.com/downloads";
const GIT_DOWNLOAD_LABEL: &str = "Download Git";

/// The too-old version this process already announced. Every window applies
/// the same runtime change; one notice is enough.
static ANNOUNCED_OUTDATED_GIT: std::sync::Mutex<Option<GitVersion>> = std::sync::Mutex::new(None);

/// The notice for a git older than GitComet supports, if `runtime` is one.
pub(super) fn outdated_git_message(runtime: &GitRuntimeState) -> Option<(GitVersion, String)> {
    let version = runtime
        .version()
        .filter(|version| !version.is_supported())?;
    Some((
        version,
        format!(
            "Git {version} is older than {}, the oldest version {} supports. \
             Some features, such as repository maintenance, are turned off. Please update Git.",
            GitVersion::MINIMUM,
            crate::view::product_name()
        ),
    ))
}

fn claim_outdated_git_notice(version: GitVersion) -> bool {
    let mut announced = ANNOUNCED_OUTDATED_GIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *announced == Some(version) {
        return false;
    }
    *announced = Some(version);
    true
}

#[cfg(test)]
pub(super) fn reset_outdated_git_notice_for_test() {
    *ANNOUNCED_OUTDATED_GIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

impl GitCometView {
    pub(super) fn maybe_warn_outdated_git(&mut self, cx: &mut gpui::Context<Self>) {
        if self.view_mode != GitCometViewMode::Normal {
            return;
        }
        let Some((version, message)) = outdated_git_message(&self.state.git_runtime) else {
            return;
        };
        if !claim_outdated_git_notice(version) {
            return;
        }
        self.toast_host.update(cx, |host, cx| {
            host.push_sticky_toast_with_link(
                components::ToastKind::Warning,
                message,
                GIT_DOWNLOAD_URL.to_string(),
                GIT_DOWNLOAD_LABEL.to_string(),
                cx,
            );
        });
    }
}
