#[test]
fn badge_feedback_remains_visible_on_transparent_controls() {
    use crate::kit::interaction::{InteractionFeedback, InteractionState};
    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        let style = worktree_badge_interaction(theme);
        for feedback in [InteractionFeedback::Hovered, InteractionFeedback::Pressed] {
            assert!(
                style
                    .fill(InteractionState::default(), feedback)
                    .unwrap()
                    .alpha
                    > 0.0
            );
        }
        assert!(
            style
                .fill(
                    InteractionState::default().open(true),
                    InteractionFeedback::Resting
                )
                .unwrap()
                .alpha
                > 0.0
        );
    }
}

/// Every worktree badge -- the branch row pill, the details chip, the log
/// row badge -- goes through one height, so they cannot drift apart.
#[test]
fn worktree_badges_grow_with_the_density() {
    use crate::appearance::{Appearance, UiDensity};
    let scale = |density| {
        ui_scale::UiScale::from_percent(100).with_appearance(Appearance {
            density,
            ..Appearance::default()
        })
    };
    let heights: Vec<_> = UiDensity::ALL.into_iter().map(scale).collect();
    for at in &heights {
        assert!(
            worktree_badge_height(*at)
                < at.row_height(
                    SIDEBAR_TREE_ROW_HEIGHT_PX,
                    SIDEBAR_TREE_COMFORTABLE_ROW_HEIGHT_PX,
                ),
            "the badge must still fit the row it sits in"
        );
    }
    assert!(
        heights
            .windows(2)
            .all(|w| worktree_badge_height(w[1]) > worktree_badge_height(w[0])),
        "the badge must grow at every density step"
    );
    let default = scale(UiDensity::default());
    assert_eq!(
        worktree_badge_height(ui_scale::UiScale::from_percent(200)),
        worktree_badge_height(default) * 2.0,
        "and follow the UI zoom"
    );
}
use super::*;
use gitcomet_core::domain::{
    Branch, Commit, CommitId, DiffTarget, LogPage, RemoteBranch, RepoSpec, Upstream,
    UpstreamDivergence, Worktree,
};
use gitcomet_core::services::{GitBackend, GitRepository, Result};
use gitcomet_state::msg::{InternalMsg, Msg};
use gitcomet_state::store::AppStore;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

struct BlockingBackend;

impl GitBackend for BlockingBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        loop {
            std::thread::park();
        }
    }
}

fn wait_until(
    cx: &mut gpui::VisualTestContext,
    description: &str,
    ready: impl Fn(&mut gpui::VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        if ready(cx) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn sync_view_for_tests(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn branch_row_index_for_name(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    section: BranchSection,
    name: &str,
) -> usize {
    cx.update(|_window, app| {
        let sidebar_pane = view.read(app).sidebar_pane.clone();
        sidebar_pane.update(app, |pane, _cx| {
            let presentation = pane
                .branch_sidebar_presentation_cached()
                .expect("expected sidebar presentation");
            presentation
                .rows
                .iter()
                .position(|row| {
                    matches!(
                        row,
                        BranchSidebarRow::Branch {
                            name: row_name,
                            section: row_section,
                            ..
                        } if *row_section == section && row_name.as_ref() == name
                    )
                })
                .unwrap_or_else(|| panic!("expected {section:?} branch row `{name}`"))
        })
    })
}

fn leak_selector(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

fn commit_id(id: &str) -> CommitId {
    CommitId(id.into())
}

/// The blocking backend never finishes opening, so declare the initial
/// walk before answering it by hand. Unsolicited replies are rejected.
fn begin_log_for_test(store: &AppStore, repo_id: RepoId) -> gitcomet_state::model::LogLoadSeq {
    let mut state = (*store.snapshot()).clone();
    let repo = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id)
        .expect("fixture repo");
    let seq = repo
        .loads_in_flight
        .request_log(gitcomet_state::model::PendingLogLoad {
            scope: repo.history_state.history_scope,
            author: None,
            limit: 200,
            cursor: None,
        })
        .expect("initial fixture walk");
    store.replace_snapshot_for_test(Arc::new(state));
    seq
}

fn commit(id: &str) -> Commit {
    Commit {
        id: commit_id(id),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: id.into(),
        author: "author".into(),
        time: SystemTime::UNIX_EPOCH,
    }
}

#[test]
fn worktree_badges_use_a_theme_surface_only_for_the_active_body() {
    let light = AppTheme::gitcomet_light();
    let dark = AppTheme::gitcomet_dark();
    for (theme, expected_active_bg) in [
        (light, light.colors.surface.canvas),
        (dark, dark.colors.surface.raised),
    ] {
        let palette = worktree_badge_palette(theme);

        assert_eq!(palette.bg.alpha, 0.0);
        assert_eq!(palette.active_bg, expected_active_bg);
        assert_eq!(palette.text, theme.colors.foreground.emphasis);

        let closed = worktree_badge_colors(palette, false, false);
        assert_eq!(closed.bg, palette.bg);
        assert_eq!(closed.border, palette.border);
        assert_eq!(closed.icon, palette.icon);
        assert_eq!(closed.text, palette.text);

        let open = worktree_badge_colors(palette, true, false);
        assert_eq!(open.bg, palette.active_bg);
        assert_eq!(open.border, palette.open_border);
        assert_eq!(open.icon, palette.open_icon);
        assert_eq!(open.text, palette.open_text);

        assert_eq!(upstream_badge_colors(palette), open);

        let menu_active = worktree_badge_colors(palette, true, true);
        assert_eq!(menu_active.border, palette.active_border);
        assert_eq!(menu_active.icon, palette.active_icon);
        assert_eq!(menu_active.text, palette.active_text);
    }
}

#[test]
fn worktree_origin_label_pairs_branch_with_folder() {
    assert_eq!(
        worktree_origin_label(Some("dev"), false, std::path::Path::new("/home/u/GitComet")),
        "dev · GitComet"
    );
}

#[test]
fn worktree_origin_label_names_a_detached_worktree_by_its_folder() {
    assert_eq!(
        worktree_origin_label(None, true, std::path::Path::new("/home/u/GitComet2")),
        "(detached) · GitComet2"
    );
}

#[test]
fn worktree_origin_label_falls_back_when_one_half_is_missing() {
    // No branch and no detached marker: the folder alone identifies it.
    assert_eq!(
        worktree_origin_label(None, false, std::path::Path::new("/home/u/GitComet3")),
        "GitComet3"
    );
    // A root path has no final component to name.
    assert_eq!(
        worktree_origin_label(Some("main"), false, std::path::Path::new("/")),
        "main"
    );
}

#[test]
fn worktree_branch_badge_label_prefers_open_repo_head_branch() {
    let listed: SharedString = "feature/listed".into();
    let mut open_repo = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo-feature"),
        },
    );
    open_repo.head_branch = Loadable::Ready("feature/live".to_string());

    let label = worktree_branch_badge_label(Some(&listed), false, Some(&open_repo))
        .expect("expected live branch badge label");
    assert_eq!(label.as_ref(), "feature/live");
}

#[test]
fn worktree_branch_badge_label_reports_detached_open_repo() {
    let listed: SharedString = "feature/listed".into();
    let mut open_repo = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo-feature"),
        },
    );
    open_repo.head_branch = Loadable::Ready("HEAD".to_string());
    open_repo.detached_head_commit = Some(commit_id("detached"));

    let label = worktree_branch_badge_label(Some(&listed), false, Some(&open_repo))
        .expect("expected detached branch badge label");
    assert_eq!(label.as_ref(), "(detached)");
}

#[test]
fn listed_worktree_paths_by_branch_includes_closed_worktrees() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![
        Worktree {
            path: std::path::PathBuf::from("/tmp/repo"),
            head: None,
            branch: Some("main".to_string()),
            detached: false,
        },
        Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        },
        Worktree {
            path: std::path::PathBuf::from("/tmp/repo-detached"),
            head: None,
            branch: None,
            detached: true,
        },
    ]));

    let paths = listed_worktree_paths_by_branch(&repo);

    assert_eq!(
        paths.get("feature"),
        Some(&std::path::PathBuf::from("/tmp/repo-feature"))
    );
    assert!(!paths.contains_key("main"));
    assert!(!paths.contains_key("repo-detached"));
}

#[test]
fn listed_worktree_paths_by_branch_prefers_first_branch_match() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![
        Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature-a"),
            head: None,
            branch: Some("feature/shared".to_string()),
            detached: false,
        },
        Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature-b"),
            head: None,
            branch: Some("feature/shared".to_string()),
            detached: false,
        },
    ]));

    let paths = listed_worktree_paths_by_branch(&repo);

    assert_eq!(
        paths.get("feature/shared"),
        Some(&std::path::PathBuf::from("/tmp/repo-feature-a"))
    );
}

#[test]
fn listed_worktree_paths_returns_empty_when_worktrees_loading() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Loading;

    let paths = listed_worktree_paths_by_branch(&repo);

    assert!(paths.is_empty());
}

#[test]
fn listed_worktree_paths_returns_empty_when_worktrees_not_loaded() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::NotLoaded;

    let paths = listed_worktree_paths_by_branch(&repo);

    assert!(paths.is_empty());
}

#[test]
fn listed_worktree_paths_returns_empty_when_worktrees_error() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Error("failed to load".into());

    let paths = listed_worktree_paths_by_branch(&repo);

    assert!(paths.is_empty());
}

#[test]
fn listed_worktree_paths_returns_empty_when_no_worktrees() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![]));

    let paths = listed_worktree_paths_by_branch(&repo);

    assert!(paths.is_empty());
}

#[test]
fn active_worktree_paths_by_branch_only_includes_open_worktrees() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![
        Worktree {
            path: std::path::PathBuf::from("/tmp/repo"),
            head: None,
            branch: Some("main".to_string()),
            detached: false,
        },
        Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        },
        Worktree {
            path: std::path::PathBuf::from("/tmp/repo-detached"),
            head: None,
            branch: None,
            detached: true,
        },
    ]));

    let mut open_main = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    open_main.head_branch = Loadable::Ready("main".to_string());
    let mut open_feature = RepoState::new_opening(
        RepoId(3),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature"),
        },
    );
    open_feature.head_branch = Loadable::Ready("feature".to_string());

    let active = active_worktree_paths_by_branch(&repo, &[open_main, open_feature]);

    assert_eq!(
        active.get("main"),
        Some(&std::path::PathBuf::from("/tmp/repo"))
    );
    assert_eq!(
        active.get("feature"),
        Some(&std::path::PathBuf::from("/tmp/repo-feature"))
    );
    assert!(!active.contains_key("repo-detached"));
}

#[test]
fn active_worktree_paths_by_branch_skips_closed_worktrees() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: std::path::PathBuf::from("/tmp/repo-feature"),
        head: None,
        branch: Some("feature".to_string()),
        detached: false,
    }]));

    let active = active_worktree_paths_by_branch(&repo, &[]);

    assert!(active.is_empty());
}

#[test]
fn active_worktree_paths_by_branch_uses_open_repo_head_branch_for_live_updates() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: std::path::PathBuf::from("/tmp/repo-feature"),
        head: None,
        branch: Some("feature/old".to_string()),
        detached: false,
    }]));

    let mut open_worktree = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature"),
        },
    );
    open_worktree.head_branch = Loadable::Ready("feature/new".to_string());
    open_worktree.head_branch_rev = 1;

    let active = active_worktree_paths_by_branch(&repo, &[open_worktree]);

    assert!(!active.contains_key("feature/old"));
    assert_eq!(
        active.get("feature/new"),
        Some(&std::path::PathBuf::from("/tmp/repo-feature"))
    );
}

#[test]
fn active_worktree_paths_by_branch_falls_back_to_listed_branch_while_head_is_loading() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: std::path::PathBuf::from("/tmp/repo-feature"),
        head: None,
        branch: Some("feature/listed".to_string()),
        detached: false,
    }]));

    let open_worktree = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature"),
        },
    );

    let active = active_worktree_paths_by_branch(&repo, &[open_worktree]);

    assert_eq!(
        active.get("feature/listed"),
        Some(&std::path::PathBuf::from("/tmp/repo-feature"))
    );
}

#[test]
fn active_worktree_paths_by_branch_hides_detached_open_worktrees() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: std::path::PathBuf::from("/tmp/repo-feature"),
        head: None,
        branch: Some("feature/old".to_string()),
        detached: false,
    }]));

    let mut open_worktree = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature"),
        },
    );
    open_worktree.head_branch = Loadable::Ready("HEAD".to_string());
    open_worktree.head_branch_rev = 1;
    open_worktree.detached_head_commit = Some(CommitId("deadbeef".into()));

    let active = active_worktree_paths_by_branch(&repo, &[open_worktree]);

    assert!(active.is_empty());
}

#[test]
fn active_worktree_paths_by_branch_keeps_first_listed_worktree_for_branch() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![
        Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature-a"),
            head: None,
            branch: Some("feature/shared".to_string()),
            detached: false,
        },
        Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature-b"),
            head: None,
            branch: Some("feature/shared".to_string()),
            detached: false,
        },
    ]));

    let mut open_first = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature-a"),
        },
    );
    open_first.head_branch = Loadable::Ready("feature/shared".to_string());

    let mut open_second = RepoState::new_opening(
        RepoId(3),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature-b"),
        },
    );
    open_second.head_branch = Loadable::Ready("feature/shared".to_string());

    let active = active_worktree_paths_by_branch(&repo, &[open_first, open_second]);

    assert_eq!(
        active.get("feature/shared"),
        Some(&std::path::PathBuf::from("/tmp/repo-feature-a"))
    );
}

#[test]
fn active_worktree_paths_returns_empty_when_worktrees_loading() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Loading;

    let open_repo = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature"),
        },
    );

    let active = active_worktree_paths_by_branch(&repo, &[open_repo]);

    assert!(active.is_empty());
}

#[test]
fn active_worktree_paths_returns_empty_when_worktrees_not_loaded() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::NotLoaded;

    let open_repo = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature"),
        },
    );

    let active = active_worktree_paths_by_branch(&repo, &[open_repo]);

    assert!(active.is_empty());
}

#[test]
fn active_worktree_paths_returns_empty_when_worktrees_error() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Error("failed to load".into());

    let open_repo = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature"),
        },
    );

    let active = active_worktree_paths_by_branch(&repo, &[open_repo]);

    assert!(active.is_empty());
}

#[test]
fn active_worktree_paths_returns_empty_when_worktrees_empty() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![]));

    let open_repo = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature"),
        },
    );

    let active = active_worktree_paths_by_branch(&repo, &[open_repo]);

    assert!(active.is_empty());
}

#[test]
fn active_worktree_paths_matches_open_repo_by_workdir_path() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: std::path::PathBuf::from("/tmp/repo-feature"),
        head: None,
        branch: Some("feature/listed".to_string()),
        detached: false,
    }]));

    let mut open_repo = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo-feature"),
        },
    );
    open_repo.head_branch = Loadable::Ready("different-branch".to_string());

    let active = active_worktree_paths_by_branch(&repo, &[open_repo]);

    assert_eq!(
        active.get("different-branch"),
        Some(&std::path::PathBuf::from("/tmp/repo-feature"))
    );
    assert!(!active.contains_key("feature/listed"));
}

#[test]
fn branch_worktree_badge_path_prefers_listed_worktree_and_falls_back_to_active() {
    assert_eq!(
        branch_worktree_badge_path(
            Some(std::path::Path::new("/tmp/repo-feature-listed")),
            Some(std::path::Path::new("/tmp/repo-feature-open")),
        ),
        Some(std::path::PathBuf::from("/tmp/repo-feature-listed"))
    );
    assert_eq!(
        branch_worktree_badge_path(None, Some(std::path::Path::new("/tmp/repo-feature-open")),),
        Some(std::path::PathBuf::from("/tmp/repo-feature-open"))
    );
}

#[test]
fn local_branch_double_click_checks_out_when_no_worktree_is_open() {
    assert_eq!(
        local_branch_double_click_action("feature/workspace", None),
        LocalBranchDoubleClickAction::CheckoutBranch {
            name: "feature/workspace".to_string(),
        }
    );
}

#[test]
fn local_branch_double_click_opens_worktree_when_branch_has_active_worktree() {
    assert_eq!(
        local_branch_double_click_action(
            "feature/workspace",
            Some(std::path::Path::new("/tmp/repo-feature"))
        ),
        LocalBranchDoubleClickAction::OpenWorktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
        }
    );
}

#[test]
fn branch_row_selection_requires_matching_clicked_branch_identity() {
    let target = commit_id("shared-tip");
    let selected_branch = SelectedBranch {
        repo_id: RepoId(1),
        target: BranchMenuTarget::local("main"),
    };
    let local_main = BranchMenuTarget::local("main");
    let remote_main = BranchMenuTarget::remote("origin", "main");

    assert!(branch_row_is_selected(
        Some(&selected_branch),
        RepoId(1),
        &local_main,
        Some(&target),
        Some(&target)
    ));
    assert!(!branch_row_is_selected(
        Some(&selected_branch),
        RepoId(1),
        &remote_main,
        Some(&target),
        Some(&target)
    ));
}

#[test]
fn branch_row_selection_requires_matching_selected_commit() {
    let target = commit_id("main-tip");
    let other = commit_id("other-tip");
    let selected_branch = SelectedBranch {
        repo_id: RepoId(1),
        target: BranchMenuTarget::local("main"),
    };
    let local_main = BranchMenuTarget::local("main");

    assert!(!branch_row_is_selected(
        Some(&selected_branch),
        RepoId(1),
        &local_main,
        Some(&other),
        Some(&target)
    ));
    assert!(!branch_row_is_selected(
        Some(&selected_branch),
        RepoId(1),
        &local_main,
        None,
        Some(&target)
    ));
}

#[test]
fn branch_row_selection_requires_resolved_selected_branch_tip() {
    let target = commit_id("main-tip");
    let selected_branch = SelectedBranch {
        repo_id: RepoId(1),
        target: BranchMenuTarget::local("main"),
    };
    let local_main = BranchMenuTarget::local("main");

    assert!(!branch_row_is_selected(
        Some(&selected_branch),
        RepoId(1),
        &local_main,
        Some(&target),
        None
    ));
}

#[test]
fn branch_click_history_reveal_target_switches_head_local_branch_to_full_reachable() {
    let target = commit_id("main-tip");
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.history_state.history_scope = LogScope::CurrentBranch;
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "main".to_string(),
        target: target.clone(),
        upstream: None,
        divergence: None,
    }]));

    assert_eq!(
        branch_click_history_reveal_target(&repo, &BranchMenuTarget::local("main"), true,),
        Some(BranchHistoryRevealTarget {
            commit_id: target,
            fallback_scope: Some(LogScope::FullReachable),
        })
    );
}

#[test]
fn branch_click_history_reveal_target_switches_non_head_local_branch_to_all_branches() {
    let target = commit_id("feature-tip");
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.history_state.history_scope = LogScope::CurrentBranch;
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature".to_string(),
        target: target.clone(),
        upstream: None,
        divergence: None,
    }]));

    assert_eq!(
        branch_click_history_reveal_target(&repo, &BranchMenuTarget::local("feature"), false,),
        Some(BranchHistoryRevealTarget {
            commit_id: target,
            fallback_scope: Some(LogScope::AllBranches),
        })
    );
}

#[test]
fn branch_click_history_reveal_target_switches_remote_branch_to_all_branches() {
    let target = commit_id("origin-feature-tip");
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.history_state.history_scope = LogScope::CurrentBranch;
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "feature/topic".to_string(),
        target: target.clone(),
    }]));

    assert_eq!(
        branch_click_history_reveal_target(
            &repo,
            &BranchMenuTarget::remote("origin", "feature/topic"),
            false,
        ),
        Some(BranchHistoryRevealTarget {
            commit_id: target,
            fallback_scope: Some(LogScope::AllBranches),
        })
    );
}

#[test]
fn branch_click_history_reveal_target_preserves_a_slash_remote_identity() {
    let selected_target = commit_id("team-alice-main-tip");
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "team".to_string(),
            name: "alice/main".to_string(),
            target: commit_id("team-alice-main-as-a-branch"),
        },
        RemoteBranch {
            remote: "team/alice".to_string(),
            name: "main".to_string(),
            target: selected_target.clone(),
        },
    ]));

    assert_eq!(
        branch_click_history_reveal_target(
            &repo,
            &BranchMenuTarget::remote("team/alice", "main"),
            false,
        ),
        Some(BranchHistoryRevealTarget {
            commit_id: selected_target,
            fallback_scope: Some(LogScope::AllBranches),
        })
    );
}

#[gpui::test]
fn branch_badges_are_static_and_worktree_badge_remains_interactive(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    // OpenRepo normalizes paths. Give it and the worktree list the same
    // native absolute paths, including on Windows and symlinked temp dirs.
    let worktrees_dir = tempfile::tempdir().expect("worktree fixture directory");
    let worktrees_path =
        gitcomet_core::path_utils::canonicalize_or_original(worktrees_dir.path().to_path_buf());
    let repo_path = worktrees_path.join("repo");
    let feature_path = worktrees_path.join("repo-feature");
    std::fs::create_dir(&repo_path).expect("main worktree directory");
    std::fs::create_dir(&feature_path).expect("feature worktree directory");
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    crate::view::test_support::redraw(cx);
    store_for_assert.dispatch(Msg::OpenRepo(repo_path.clone()));
    wait_until(cx, "opened repo placeholder", |_cx| {
        let snapshot = store_for_assert.snapshot();
        snapshot.active_repo == Some(repo_id)
            && snapshot.repos.iter().any(|repo| repo.id == repo_id)
    });
    sync_view_for_tests(cx, &view);

    store_for_assert.dispatch(Msg::Internal(InternalMsg::HeadBranchLoaded {
        repo_id,
        result: Ok("main".to_string()),
    }));
    store_for_assert.dispatch(Msg::Internal(InternalMsg::BranchesLoaded {
        repo_id,
        result: Ok(vec![
            Branch {
                name: "main".to_string(),
                target: commit_id("main-tip"),
                upstream: Some(Upstream {
                    remote: "origin".to_string(),
                    branch: "main".to_string(),
                }),
                divergence: None,
            },
            Branch {
                name: "feature".to_string(),
                target: commit_id("feature-tip"),
                upstream: Some(Upstream {
                    remote: "origin".to_string(),
                    branch: "feature".to_string(),
                }),
                divergence: Some(UpstreamDivergence {
                    ahead: 3,
                    behind: 2,
                }),
            },
        ]),
    }));
    store_for_assert.dispatch(Msg::Internal(InternalMsg::RemoteBranchesLoaded {
        repo_id,
        result: Ok(vec![
            RemoteBranch {
                remote: "origin".to_string(),
                name: "main".to_string(),
                target: commit_id("origin-main-tip"),
            },
            RemoteBranch {
                remote: "origin".to_string(),
                name: "feature".to_string(),
                target: commit_id("origin-feature-tip"),
            },
        ]),
    }));
    store_for_assert.dispatch(Msg::Internal(InternalMsg::WorktreesLoaded {
        repo_id,
        result: Ok(vec![
            Worktree {
                path: repo_path,
                head: None,
                branch: Some("main".to_string()),
                detached: false,
            },
            Worktree {
                path: feature_path.clone(),
                head: None,
                branch: Some("feature".to_string()),
                detached: false,
            },
        ]),
    }));
    wait_until(cx, "sidebar badges loaded", |_cx| {
        let snapshot = store_for_assert.snapshot();
        let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
            return false;
        };
        matches!(repo.head_branch, Loadable::Ready(ref head) if head == "main")
            && matches!(repo.branches, Loadable::Ready(_))
            && matches!(repo.remote_branches, Loadable::Ready(_))
            && matches!(repo.worktrees, Loadable::Ready(_))
    });
    sync_view_for_tests(cx, &view);

    let feature_ix = branch_row_index_for_name(cx, &view, BranchSection::Local, "feature");
    let upstream_ix = branch_row_index_for_name(cx, &view, BranchSection::Remote, "origin/main");

    let feature_row_selector = leak_selector(format!("branch_row_{}_{}", repo_id.0, feature_ix));
    let feature_badge_selector = leak_selector(format!("branch_worktree_badge_{feature_ix}"));
    let main_ix = branch_row_index_for_name(cx, &view, BranchSection::Local, "main");
    let main_badge = leak_selector(format!("branch_worktree_badge_{main_ix}"));
    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        cx.update(|_, app| view.update(app, |this, cx| this.set_theme(theme, cx)));
        cx.simulate_mouse_move(
            point(px(600.0), px(400.0)),
            None,
            gpui::Modifiers::default(),
        );
        crate::view::test_support::redraw(cx);
        let main_resting = crate::test_support::painted_control_quads(cx, main_badge);
        assert!(
            main_resting.iter().any(|(bg, _)| bg.as_solid()
                == Some(palette::IntoColor::into_color(
                    worktree_badge_palette(theme).active_bg
                ))),
            "open workspace must retain its resting fill"
        );
        let closed_resting = crate::test_support::painted_control_quads(cx, feature_badge_selector);
        let position = cx.debug_bounds(feature_badge_selector).unwrap().center();
        cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
        crate::view::test_support::redraw(cx);
        assert_ne!(
            crate::test_support::painted_control_quads(cx, feature_badge_selector),
            closed_resting,
            "hover must be visible"
        );
        cx.simulate_mouse_down(
            position,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        crate::view::test_support::redraw(cx);
        assert_ne!(
            crate::test_support::painted_control_quads(cx, feature_badge_selector),
            closed_resting,
            "press must be visible"
        );
        cx.simulate_mouse_up(
            point(px(600.0), px(400.0)),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(600.0), px(400.0)),
            None,
            gpui::Modifiers::default(),
        );
        crate::view::test_support::redraw(cx);
        assert_eq!(
            crate::test_support::painted_control_quads(cx, feature_badge_selector),
            closed_resting
        );
        assert_eq!(
            crate::test_support::painted_control_quads(cx, main_badge),
            main_resting
        );
    }
    let feature_pull_badge_selector = leak_selector(format!("branch_pull_badge_{feature_ix}"));
    let feature_push_badge_selector = leak_selector(format!("branch_push_badge_{feature_ix}"));
    let feature_menu_selector = leak_selector(format!(
        "branch_menu_indicator_{}_{}",
        repo_id.0, feature_ix
    ));
    assert!(
        cx.debug_bounds(feature_menu_selector).is_none(),
        "expected branch hamburger menu indicator to be removed"
    );
    let feature_badge_before = cx
        .debug_bounds(feature_badge_selector)
        .expect("expected worktree badge before hover");
    let feature_pull_badge_before = cx
        .debug_bounds(feature_pull_badge_selector)
        .expect("expected pull count badge before hover");
    let feature_push_badge_before = cx
        .debug_bounds(feature_push_badge_selector)
        .expect("expected push count badge before hover");
    let feature_row_bounds = cx
        .debug_bounds(feature_row_selector)
        .expect("expected feature branch row");
    let feature_row_center = feature_row_bounds.center();
    let feature_dots_selector = leak_selector(format!("branch_dots_{feature_ix}"));
    assert!(
        cx.debug_bounds(feature_dots_selector).is_none(),
        "expected the trailing `⋮` slot to be gone from branch rows"
    );
    // Backgrounds reach the panel edge; content retains its inset plus
    // trailing padding so the overlay scrollbar cannot cover the badge.
    assert!(
        (feature_row_bounds.right()
            - feature_badge_before.right()
            - px(components::ROW_HIGHLIGHT_INSET_PX + 4.0))
        .abs()
            <= px(1.0),
        "expected the worktree badge to sit one trailing pad off the row's right edge, \
             row right {:?} badge right {:?}",
        feature_row_bounds.right(),
        feature_badge_before.right()
    );
    cx.simulate_mouse_move(feature_row_center, None, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    let feature_badge_after = cx
        .debug_bounds(feature_badge_selector)
        .expect("expected worktree badge after hover");
    let feature_pull_badge_after = cx
        .debug_bounds(feature_pull_badge_selector)
        .expect("expected pull count badge after hover");
    let feature_push_badge_after = cx
        .debug_bounds(feature_push_badge_selector)
        .expect("expected push count badge after hover");
    // Hover reveals nothing in the trailing run any more, so the badges
    // keep the exact geometry they had at rest.
    assert!(
        cx.debug_bounds(feature_dots_selector).is_none(),
        "expected no `⋮` slot to appear on row hover"
    );
    assert_eq!(
        feature_badge_before.left(),
        feature_badge_after.left(),
        "expected the worktree badge to stay fixed on row hover"
    );
    assert_eq!(
        feature_badge_before.right(),
        feature_badge_after.right(),
        "expected the worktree badge to stay fixed on row hover"
    );
    assert_eq!(
        feature_pull_badge_before.left(),
        feature_pull_badge_after.left(),
        "expected the pull badge to stay fixed on row hover"
    );
    assert_eq!(
        feature_push_badge_before.left(),
        feature_push_badge_after.left(),
        "expected the push badge to stay fixed on row hover"
    );
    // Right-click over the label (near the row's leading edge) rather than
    // the center: the trailing area holds the worktree badge, which opens
    // its own menu.
    let feature_row_label_point =
        gpui::point(feature_row_bounds.left() + px(48.0), feature_row_center.y);
    cx.simulate_mouse_down(
        feature_row_label_point,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    assert!(!cx.update(|_, app| view.read(app).popover_host.read(app).is_open()));
    cx.simulate_mouse_up(
        feature_row_label_point,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    crate::view::test_support::redraw(cx);
    let popover_kind = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert!(
        matches!(
            popover_kind,
            Some(PopoverKind::BranchMenu {
                repo_id: opened_repo_id,
                target: BranchMenuTarget::Local { ref name },
                ..
            }) if opened_repo_id == repo_id && name == "feature"
        ),
        "expected feature branch right-click to open the branch menu"
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.close_popover(cx);
            });
        });
    });
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);

    let feature_badge_center = cx
        .debug_bounds(feature_badge_selector)
        .expect("expected worktree badge before badge click")
        .center();
    cx.simulate_mouse_move(feature_badge_center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        feature_badge_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        feature_badge_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    crate::view::test_support::redraw(cx);
    let popover_kind = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert!(
        matches!(
            popover_kind,
            Some(PopoverKind::Repo {
                repo_id: opened_repo_id,
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::Menu {
                    ref path,
                    branch: Some(ref branch),
                }),
            }) if opened_repo_id == repo_id
                && path == &feature_path
                && branch == "feature"
        ),
        "expected worktree badge click to open the worktree menu"
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.close_popover(cx);
            });
        });
    });
    cx.run_until_parked();

    cx.simulate_mouse_down(
        feature_badge_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        feature_badge_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    crate::view::test_support::redraw(cx);
    let popover_kind = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert!(
        matches!(
            popover_kind,
            Some(PopoverKind::Repo {
                repo_id: opened_repo_id,
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::Menu {
                    ref path,
                    branch: Some(ref branch),
                }),
            }) if opened_repo_id == repo_id
                && path == &feature_path
                && branch == "feature"
        ),
        "expected worktree badge right-click to open the worktree menu"
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.close_popover(cx);
            });
        });
    });
    cx.run_until_parked();

    let upstream_row_selector = leak_selector(format!("branch_row_{}_{}", repo_id.0, upstream_ix));
    let upstream_badge_selector = leak_selector(format!("branch_upstream_badge_{upstream_ix}"));
    let upstream_menu_selector = leak_selector(format!(
        "branch_menu_indicator_{}_{}",
        repo_id.0, upstream_ix
    ));
    assert!(
        cx.debug_bounds(upstream_menu_selector).is_none(),
        "expected upstream branch hamburger menu indicator to be removed"
    );
    let upstream_badge_before = cx
        .debug_bounds(upstream_badge_selector)
        .expect("expected upstream badge before hover");
    let upstream_row_center = cx
        .debug_bounds(upstream_row_selector)
        .expect("expected upstream branch row")
        .center();
    cx.simulate_mouse_move(upstream_row_center, None, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    let upstream_badge_after = cx
        .debug_bounds(upstream_badge_selector)
        .expect("expected upstream badge after hover");
    assert_eq!(
        upstream_badge_before.left(),
        upstream_badge_after.left(),
        "expected the upstream badge to stay fixed on row hover"
    );
    assert_eq!(
        upstream_badge_before.right(),
        upstream_badge_after.right(),
        "expected the upstream badge to stay fixed on row hover"
    );
}

#[gpui::test]
fn branch_reveal_marks_the_branch_chip_on_the_revealed_history_row(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let feature_tip = commit_id("feature-tip");
    let initial_scope = LogScope::default();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/repo")));
    wait_until(cx, "opened repo placeholder", |_cx| {
        let snapshot = store_for_assert.snapshot();
        snapshot.active_repo == Some(repo_id)
            && snapshot.repos.iter().any(|repo| repo.id == repo_id)
    });

    let log_seq = begin_log_for_test(&store_for_assert, repo_id);
    store_for_assert.dispatch(Msg::Internal(InternalMsg::HeadBranchLoaded {
        repo_id,
        result: Ok("main".to_string()),
    }));
    store_for_assert.dispatch(Msg::Internal(InternalMsg::BranchesLoaded {
        repo_id,
        result: Ok(vec![
            Branch {
                name: "main".to_string(),
                target: commit_id("main-tip"),
                upstream: None,
                divergence: None,
            },
            Branch {
                name: "feature".to_string(),
                target: feature_tip.clone(),
                upstream: None,
                divergence: None,
            },
        ]),
    }));
    store_for_assert.dispatch(Msg::Internal(InternalMsg::LogLoaded {
        repo_id,
        seq: log_seq,
        scope: initial_scope,
        cursor: None,
        result: Ok(std::sync::Arc::new(LogPage {
            commits: vec![commit("feature-tip"), commit("main-tip")],
            next_cursor: None,
        })
        .into()),
    }));
    wait_until(cx, "sidebar repo data", |cx| {
        sync_view_for_tests(cx, &view);
        let snapshot = store_for_assert.snapshot();
        let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
            return false;
        };
        matches!(repo.branches, Loadable::Ready(_)) && matches!(repo.log, Loadable::Ready(_))
    });

    let sidebar_pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    cx.update(|window, app| {
        sidebar_pane.update(app, |pane, cx| {
            pane.reveal_branch_commit_in_history(
                repo_id,
                BranchMenuTarget::local("feature"),
                feature_tip.clone(),
                None,
                cx,
            );
        });
        let _ = window.draw(app);
    });

    wait_until(cx, "revealed branch tip selected", |cx| {
        sync_view_for_tests(cx, &view);
        let snapshot = store_for_assert.snapshot();
        snapshot
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| repo.history_state.selected_commit.as_ref() == Some(&feature_tip))
    });

    let marked = cx.update(|_window, app| {
        let history_view = view.read(app).main_pane.read(app).history_view.clone();
        history_view
            .read(app)
            .selected_branch_for_history_row(repo_id, true)
    });
    assert_eq!(
        marked,
        Some(SelectedHistoryBranch {
            target: BranchMenuTarget::local("feature"),
        }),
        "the revealed row should mark the clicked branch's chip as selected"
    );
}

#[gpui::test]
fn branch_reveal_routes_through_main_pane_and_selects_commit(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let sync_view_from_store = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            window.refresh();
            let _ = window.draw(app);
        });
    };

    let repo_id = RepoId(1);
    let target = commit_id("main-tip");
    let initial_scope = LogScope::default();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/repo")));
    wait_until(cx, "opened repo placeholder", |_cx| {
        let snapshot = store_for_assert.snapshot();
        snapshot.active_repo == Some(repo_id)
            && snapshot.repos.iter().any(|repo| repo.id == repo_id)
    });
    sync_view_from_store(cx);

    let log_seq = begin_log_for_test(&store_for_assert, repo_id);
    store_for_assert.dispatch(Msg::Internal(InternalMsg::HeadBranchLoaded {
        repo_id,
        result: Ok("main".to_string()),
    }));
    store_for_assert.dispatch(Msg::Internal(InternalMsg::BranchesLoaded {
        repo_id,
        result: Ok(vec![Branch {
            name: "main".to_string(),
            target: target.clone(),
            upstream: None,
            divergence: None,
        }]),
    }));
    store_for_assert.dispatch(Msg::Internal(InternalMsg::LogLoaded {
        repo_id,
        seq: log_seq,
        scope: initial_scope,
        cursor: None,
        result: Ok(std::sync::Arc::new(LogPage {
            commits: vec![commit("main-tip")],
            next_cursor: None,
        })
        .into()),
    }));
    store_for_assert.dispatch(Msg::SelectDiff {
        repo_id,
        target: DiffTarget::commit(commit_id("previous"), "previous.txt".into()),
    });
    wait_until(cx, "sidebar repo data", |_cx| {
        let snapshot = store_for_assert.snapshot();
        let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
            return false;
        };
        matches!(repo.head_branch, Loadable::Ready(ref head) if head == "main")
            && matches!(repo.branches, Loadable::Ready(_))
            && matches!(repo.log, Loadable::Ready(_))
            && repo.diff_state.diff_target.is_some()
    });
    sync_view_from_store(cx);

    wait_until(cx, "history view active repo", |cx| {
        sync_view_for_tests(cx, &view);
        cx.update(|_window, app| {
            let (sidebar_pane, main_pane) = {
                let root = view.read(app);
                (root.sidebar_pane.clone(), root.main_pane.clone())
            };
            let history_view = main_pane.read(app).history_view.clone();

            sidebar_pane.read(app).active_repo_id() == Some(repo_id)
                && main_pane.read(app).active_repo_id() == Some(repo_id)
                && history_view.read(app).active_repo_id() == Some(repo_id)
        })
    });

    sync_view_for_tests(cx, &view);
    let sidebar_pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    cx.update(|window, app| {
        sidebar_pane.update(app, |pane, cx| {
            pane.reveal_branch_commit_in_history(
                repo_id,
                BranchMenuTarget::local("main"),
                target.clone(),
                None,
                cx,
            );
        });
        let _ = window.draw(app);
    });

    wait_until(cx, "branch reveal store state", |_cx| {
        let snapshot = store_for_assert.snapshot();
        let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
            return false;
        };
        repo.diff_state.diff_target.is_none()
            && repo.history_state.history_scope == initial_scope
            && repo.history_state.selected_commit.as_ref() == Some(&target)
    });
}

#[gpui::test]
fn branch_reveal_closes_open_history_refs_hover_without_reentrant_root_update(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let sync_view_from_store = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            window.refresh();
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    };

    let repo_id = RepoId(1);
    let target = commit_id("main-tip");
    let initial_scope = LogScope::default();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/repo")));
    wait_until(cx, "opened repo placeholder", |_cx| {
        let snapshot = store_for_assert.snapshot();
        snapshot.active_repo == Some(repo_id)
            && snapshot.repos.iter().any(|repo| repo.id == repo_id)
    });
    sync_view_from_store(cx);

    let log_seq = begin_log_for_test(&store_for_assert, repo_id);
    store_for_assert.dispatch(Msg::Internal(InternalMsg::HeadBranchLoaded {
        repo_id,
        result: Ok("main".to_string()),
    }));
    store_for_assert.dispatch(Msg::Internal(InternalMsg::BranchesLoaded {
        repo_id,
        result: Ok(vec![Branch {
            name: "main".to_string(),
            target: target.clone(),
            upstream: None,
            divergence: None,
        }]),
    }));
    store_for_assert.dispatch(Msg::Internal(InternalMsg::LogLoaded {
        repo_id,
        seq: log_seq,
        scope: initial_scope,
        cursor: None,
        result: Ok(std::sync::Arc::new(LogPage {
            commits: vec![commit("main-tip")],
            next_cursor: None,
        })
        .into()),
    }));
    wait_until(cx, "sidebar repo data", |_cx| {
        let snapshot = store_for_assert.snapshot();
        let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
            return false;
        };
        matches!(repo.head_branch, Loadable::Ready(ref head) if head == "main")
            && matches!(repo.branches, Loadable::Ready(_))
            && matches!(repo.log, Loadable::Ready(_))
    });
    sync_view_from_store(cx);

    wait_until(cx, "history row rendered", |cx| {
        sync_view_for_tests(cx, &view);
        cx.debug_bounds("history_row_0").is_some()
    });

    let history_row_bounds = cx
        .debug_bounds("history_row_0")
        .expect("history row should be rendered");
    let hover_items: Arc<[HistoryRefListItem]> = vec![HistoryRefListItem {
        text: HistoryTextVm::new("main".into()),
        kind: HistoryRefListItemKind::LocalBranch {
            name: "main".to_string(),
        },
    }]
    .into();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.show_history_refs_hover(
                repo_id,
                target.clone(),
                history_row_bounds,
                hover_items.clone(),
                history_row_bounds.center(),
                window,
                cx,
            );
        });
        let _ = window.draw(app);
    });
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        assert!(crate::view::test_support::history_refs_hover_is_open(
            view.read(app),
            app
        ));
    });

    let sidebar_pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    cx.update(|window, app| {
        sidebar_pane.update(app, |pane, cx| {
            pane.reveal_branch_commit_in_history(
                repo_id,
                BranchMenuTarget::local("main"),
                target.clone(),
                None,
                cx,
            );
        });
        let _ = window.draw(app);
    });

    wait_until(cx, "branch reveal store state and hover closure", |_cx| {
        let snapshot = store_for_assert.snapshot();
        let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
            return false;
        };
        repo.history_state.history_scope == initial_scope
            && repo.history_state.selected_commit.as_ref() == Some(&target)
            && _cx.update(|_window, app| {
                !crate::view::test_support::history_refs_hover_is_open(view.read(app), app)
            })
    });
}
