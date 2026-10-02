//! The real host and pane/list renderers, driven by GPUI's headless window.
use super::*;
use crate::view::{
    GitCometView,
    hosted::{diff_pane::DiffPaneView, file_list::FileListView},
};
use gitcomet_extension_api::*;
use gpui::{Entity, IntoElement, Render, Window};

struct BenchExtension;
impl Extension for BenchExtension {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.benchmark").unwrap()
    }
    fn register(&self, r: &mut Registrar) {
        r.repository_view(
            "second",
            RepositoryViewDescriptor::new("Second", "icons/code.svg", |_, _, cx| {
                cx.new(|_| gpui::Empty).into()
            }),
        );
    }
}
enum RowProbe {
    Diff(Entity<DiffPaneView>, usize),
    Files(Entity<FileListView>, bool),
}
struct PaneHost {
    view: gpui::AnyView,
    probe: Option<RowProbe>,
    rows: Rc<Cell<usize>>,
}
impl Render for PaneHost {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        // Row construction must happen during draw: AnyElement allocations
        // belong to the frame arena, which is cleared after every iteration.
        match &self.probe {
            Some(RowProbe::Diff(pane, start)) => {
                self.rows.set(pane.update(cx, |pane, cx| {
                    pane.benchmark_window(*start, 200, window, cx)
                }));
                gpui::div().into_any_element()
            }
            Some(RowProbe::Files(files, decor)) => {
                self.rows
                    .set(files.update(cx, |files, cx| files.benchmark_window(*decor, cx)));
                gpui::div().into_any_element()
            }
            None => gpui::div()
                .size_full()
                .child(self.view.clone())
                .into_any_element(),
        }
    }
}

pub struct ExtensionFrameFixture {
    pane: DiffPane,
    pane_entity: Entity<DiffPaneView>,
    files: Entity<FileListView>,
    window: gpui::WindowHandle<PaneHost>,
    shell: gpui::WindowHandle<GitCometView>,
    repository: RepositoryHandle,
    snapshot: DiffSnapshot,
    rendered_rows: Rc<Cell<usize>>,
    next: usize,
    /// Eight groups over the files, regrouped under a new revision each time.
    groups: FileListGroups,
    cx: gpui::TestAppContext,
}
impl ExtensionFrameFixture {
    pub fn new(lines: usize, files: usize) -> Self {
        let mut cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            crate::view::extension_host::install(
                Registry::build(vec![Box::new(BenchExtension)]).unwrap(),
                cx,
            )
        });
        let (store, events) = gitcomet_state::store::AppStore::new_test(Arc::new(GixBackend));
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: "/tmp/extension-benchmark".into(),
            },
        );
        repo.open = Loadable::Ready(());
        let lifetime = repo.lifetime();
        let state = Arc::new(AppState {
            git_runtime: gitcomet_core::process::GitRuntimeState {
                preference: gitcomet_core::process::GitExecutablePreference::SystemPath,
                availability: gitcomet_core::process::GitExecutableAvailability::Available {
                    version_output: "git version 2.51.0".into(),
                },
            },
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..AppState::test_default()
        });
        store.replace_snapshot_for_test(state);
        let shell = cx.add_window(|window, cx| {
            GitCometView::new_with_config(
                store,
                events,
                crate::view::GitCometViewConfig {
                    workspace: crate::view::WorkspaceBootstrap::Empty,
                    ..crate::view::GitCometViewConfig::normal(None)
                },
                window,
                cx,
            )
        });
        cx.run_until_parked();
        let host = shell
            .update(&mut cx, |view, _, _| {
                view.extension_window.as_ref().unwrap().host()
            })
            .unwrap();
        let repository = RepositoryHandle::new(
            host.id(),
            RepoId(1),
            lifetime,
            "/tmp/extension-benchmark".into(),
        );
        let old: String = (0..lines)
            .map(|i| format!("let value_{i} = {i};\n"))
            .collect();
        let new = old.replace(" = ", " = 1 + ");
        let snapshot = DiffSnapshot::new("src/example.rs", old, new);
        let (pane, files, groups) = cx.update(|cx| {
            let pane = host
                .create_snapshot_pane(snapshot.clone(), DiffPaneOptions::default(), cx)
                .unwrap();
            let changes: Arc<Vec<CommitFileChange>> = Arc::new(
                (0..files)
                    .map(|i| {
                        CommitFileChange::new(
                            format!("src/group_{}/file_{i}.rs", i / 100).into(),
                            FileStatusKind::Modified,
                        )
                    })
                    .collect(),
            );
            // A hash lookup per file, as the API asks of a large list's groups.
            let group_of: rustc_hash::FxHashMap<std::path::PathBuf, usize> = changes
                .iter()
                .enumerate()
                .map(|(i, change)| (change.path.clone(), i % 8))
                .collect();
            let groups = FileListGroups::new(
                0,
                (0..8)
                    .map(|group| SharedString::from(format!("Group {group}")))
                    .collect::<Vec<_>>(),
                move |path| group_of.get(path).copied(),
            );
            let list = cx
                .new(|cx| FileListView::benchmark_snapshot(host, repository.clone(), changes, cx));
            (pane, list, groups)
        });
        let pane_entity = pane.view().downcast::<DiffPaneView>().unwrap();
        let view = pane.view();
        let rendered_rows = Rc::new(Cell::new(0));
        let window = cx.add_window(|_, _| PaneHost {
            view,
            probe: None,
            rows: rendered_rows.clone(),
        });
        let mut this = Self {
            pane,
            pane_entity,
            files,
            window,
            shell,
            repository,
            snapshot,
            rendered_rows,
            next: 0,
            groups,
            cx,
        };
        this.settle();
        this
    }
    fn settle(&mut self) {
        self.window
            .update(&mut self.cx, |host, _, cx| {
                host.probe = None;
                cx.notify();
            })
            .unwrap();
        for _ in 0..3 {
            self.cx.run_until_parked();
            self.cx
                .update_window(self.window.into(), |_, window, cx| {
                    window.draw(cx).clear(cx);
                })
                .unwrap();
            if self.cx.update(|cx| {
                self.pane_entity
                    .update(cx, |pane, cx| pane.benchmark_ready(cx))
            }) {
                return;
            }
        }
        panic!("the hosted diff did not finish its first window");
    }
    pub fn open_first_window(&mut self) -> usize {
        let host = self
            .shell
            .update(&mut self.cx, |view, _, _| {
                view.extension_window.as_ref().unwrap().host()
            })
            .unwrap();
        let pane = self.cx.update(|cx| {
            host.create_snapshot_pane(self.snapshot.clone(), DiffPaneOptions::default(), cx)
                .unwrap()
        });
        self.pane_entity = pane.view().downcast::<DiffPaneView>().unwrap();
        self.window
            .update(&mut self.cx, |window, _, cx| {
                window.view = pane.view();
                cx.notify();
            })
            .unwrap();
        self.pane = pane;
        self.settle();
        self.diff_window(0)
    }
    pub fn set_overlays(&mut self, insets: bool) {
        self.cx.update(|cx| {
            let mut marks = DiffAnnotations::new();
            for line in (1..=200).step_by(7) {
                marks = marks.with(DiffLineSide::New, line, DiffAnnotation::new(gpui::red()));
            }
            self.pane.set_annotations(marks, cx);
            if insets {
                self.pane.set_insets(
                    (1..=200)
                        .step_by(10)
                        .map(|line| {
                            DiffInset::new(DiffLineSide::New, line, ["An inline note".into()])
                        })
                        .collect(),
                    cx,
                );
            }
        });
        self.settle();
    }
    pub fn scroll_window(&mut self) -> usize {
        self.next = (self.next + 17) % 200;
        self.diff_window(self.next)
    }
    fn diff_window(&mut self, start: usize) -> usize {
        let rows = self.probe_rows(RowProbe::Diff(self.pane_entity.clone(), start));
        assert_eq!(rows, 200, "the diff benchmark must render a full window");
        rows
    }
    pub fn file_plan(&mut self) -> usize {
        self.cx
            .update(|cx| self.files.update(cx, |files, _| files.benchmark_plan(true)))
    }
    /// File-list plans built so far; a window draw that replans moves it.
    pub fn file_plan_builds(&mut self) -> u64 {
        self.cx
            .update(|cx| self.files.read(cx).benchmark_plan_builds())
    }
    /// Regroups every file, as an extension replacing its groups does;
    /// returns the grouped rows.
    pub fn file_regroup(&mut self) -> usize {
        self.groups.revision += 1;
        let groups = self.groups.clone();
        self.cx.update(|cx| {
            self.files
                .update(cx, |files, _| files.benchmark_regroup(groups))
        })
    }
    pub fn file_window(&mut self, decor_update: bool) -> usize {
        self.probe_rows(RowProbe::Files(self.files.clone(), decor_update))
    }
    fn probe_rows(&mut self, probe: RowProbe) -> usize {
        self.rendered_rows.set(0);
        self.window
            .update(&mut self.cx, |host, _, cx| {
                host.probe = Some(probe);
                cx.notify();
            })
            .unwrap();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.draw(cx).clear(cx)
            })
            .unwrap();
        self.rendered_rows.get()
    }
    pub fn switch_view(&mut self) {
        self.next ^= 1;
        let repository = self.repository.clone();
        let selected = (self.next & 1 == 1).then_some(0);
        self.shell
            .update(&mut self.cx, |view, window, cx| {
                view.select_repository_view(&repository, selected, window, cx)
            })
            .unwrap();
        self.cx.run_until_parked();
        self.cx
            .update_window(self.shell.into(), |_, window, cx| {
                window.draw(cx).clear(cx);
            })
            .unwrap();
    }
}

pub struct SettingsFrameFixture {
    window: gpui::WindowHandle<crate::view::settings_window::SettingsWindowView>,
    cx: gpui::TestAppContext,
}
impl Default for SettingsFrameFixture {
    fn default() -> Self {
        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(crate::view::settings_window::SettingsWindowView::new);
        Self { window, cx }
    }
}
impl SettingsFrameFixture {
    pub fn frame(&mut self) -> u64 {
        crate::view::perf::take_settings_pages_rendered();
        crate::view::perf::take_settings_renders();
        self.window
            .update(&mut self.cx, |_, _, cx| cx.notify())
            .unwrap();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.draw(cx).clear(cx);
            })
            .unwrap();
        let pages = crate::view::perf::take_settings_pages_rendered();
        let renders = crate::view::perf::take_settings_renders();
        assert!(renders > 0);
        assert_eq!(pages, renders, "each settings render must build one page");
        pages / renders
    }
}

/// An ordinary repository window with no installed extension registry.
pub struct EmptyExtensionFrameFixture {
    window: gpui::WindowHandle<GitCometView>,
    cx: gpui::TestAppContext,
}
impl Default for EmptyExtensionFrameFixture {
    fn default() -> Self {
        Self::new(false, 0)
    }
}
impl EmptyExtensionFrameFixture {
    fn new(annotated: bool, commits: usize) -> Self {
        let mut cx = gpui::TestAppContext::single();
        if annotated {
            cx.update(|cx| {
                crate::view::extension_host::install(
                    Registry::build(vec![Box::new(AnnotationExtension)]).unwrap(),
                    cx,
                )
            });
        }
        let (store, events) = gitcomet_state::store::AppStore::new_test(Arc::new(GixBackend));
        let mut state = AppState::test_default();
        state.git_runtime = gitcomet_core::process::GitRuntimeState {
            preference: gitcomet_core::process::GitExecutablePreference::SystemPath,
            availability: gitcomet_core::process::GitExecutableAvailability::Available {
                version_output: "git version 2.51.0".into(),
            },
        };
        let mut repository = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: "/tmp/empty-extension-benchmark".into(),
            },
        );
        repository.open = Loadable::Ready(());
        repository.history_state.log = Loadable::Ready(Arc::new(LogPage {
            commits: (0..commits)
                .map(|index| Commit {
                    id: CommitId(format!("{:040x}", commits - index).into()),
                    parent_ids: (index + 1 < commits)
                        .then(|| CommitId(format!("{:040x}", commits - index - 1).into()))
                        .into_iter()
                        .collect(),
                    summary: format!("Update file {index}").into(),
                    author: "Example".into(),
                    time: SystemTime::UNIX_EPOCH + Duration::from_secs(index as u64),
                })
                .collect(),
            next_cursor: None,
        }));
        repository.history_state.log_rev = 1;
        state.active_repo = Some(repository.id);
        state.repos.push(repository);
        store.replace_snapshot_for_test(Arc::new(state));
        let window = cx.add_window(|window, cx| {
            GitCometView::new_with_config(
                store,
                events,
                crate::view::GitCometViewConfig {
                    workspace: crate::view::WorkspaceBootstrap::Empty,
                    ..crate::view::GitCometViewConfig::normal(None)
                },
                window,
                cx,
            )
        });
        cx.run_until_parked();
        Self { window, cx }
    }
}

struct AnnotationExtension;
impl Extension for AnnotationExtension {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.annotation-benchmark").unwrap()
    }
    fn register(&self, r: &mut Registrar) {
        r.history_annotator(
            "marks",
            HistoryAnnotator::new(SlotSignal::default(), |_, _, _| {
                HistoryRowAnnotation::default()
                    .with_opacity(0.75)
                    .with_leading(RowMark::new(gpui::red()).with_glyph(RowGlyph::Text("●".into())))
                    .with_trailing(RowMark::new(gpui::green()).with_label("Checked"))
                    .with_range(HistoryRangeMark::new(gpui::blue(), false, false))
            }),
        );
    }
}

/// Paints the same history viewport with and without a descriptor annotator.
pub struct HistoryAnnotationFrameFixture {
    shell: EmptyExtensionFrameFixture,
    next: usize,
}
impl HistoryAnnotationFrameFixture {
    pub fn new(annotated: bool) -> Self {
        Self {
            shell: EmptyExtensionFrameFixture::new(annotated, 5_000),
            next: 0,
        }
    }
    pub fn frame(&mut self) {
        self.next = (self.next + 24) % 4_800;
        self.shell
            .window
            .update(&mut self.shell.cx, |view, _, cx| {
                let history = view.main_pane.read(cx).history_view.clone();
                history.update(cx, |history, cx| {
                    history
                        .history_scroll
                        .scroll_to_item(self.next, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            })
            .unwrap();
        self.shell.cx.run_until_parked();
        self.shell
            .cx
            .update_window(self.shell.window.into(), |_, window, cx| {
                window.draw(cx).clear(cx);
            })
            .unwrap();
    }
}
impl EmptyExtensionFrameFixture {
    pub fn frame(&mut self) -> u64 {
        crate::view::perf::take_extension_dispatch_calls();
        self.window
            .update(&mut self.cx, |_, _, cx| cx.notify())
            .unwrap();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.draw(cx).clear(cx);
            })
            .unwrap();
        let calls = crate::view::perf::take_extension_dispatch_calls();
        assert_eq!(calls, 0);
        calls
    }
}
