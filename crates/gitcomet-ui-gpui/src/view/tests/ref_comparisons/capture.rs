use super::*;

#[test]
#[ignore = "renders comparison rows with native fonts and a GPU; writes local measurements"]
fn comparison_rows_native_capture_and_publication_frames() {
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx = gpui::HeadlessAppContext::with_platform(
        text_system,
        Arc::new(crate::assets::GitCometAssets::default()),
        || {
            Some(Box::new(
                gpui_wgpu::WgpuHeadlessRenderer::new().expect("headless renderer"),
            ))
        },
    );
    let (state, _memory) = comparison_state();
    let state = Arc::new(state);
    let (store, _unobserved_events) = AppStore::new_test(Arc::new(TestBackend));
    store.disable_repo_monitors_for_test();
    store.replace_snapshot_for_test(state.clone());
    let (_quiet, events) = smol::channel::unbounded();
    let handle = cx
        .open_window(gpui::size(px(1600.0), px(1000.0)), |window, app| {
            app.new(|cx| GitCometView::new(store.clone(), events, None, window, cx))
        })
        .unwrap();
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ref-comparison-preview");
    std::fs::create_dir_all(&directory).unwrap();
    for (name, percent, width) in [
        ("narrow", 100, 300.0),
        ("wide", 100, 500.0),
        ("large-text", 150, 450.0),
        ("light", 100, 300.0),
        ("pending", 100, 300.0),
        ("unavailable", 100, 300.0),
    ] {
        let mut snapshot = state.as_ref().clone();
        match name {
            "pending" => snapshot.repos[0].ref_comparisons.target = Loadable::Loading,
            "unavailable" => snapshot.repos[0].ref_comparisons.target = Loadable::Error("The locally recorded origin/HEAD is unavailable. Choose an explicit comparison ref.".into()),
            _ => {}
        }
        let snapshot = Arc::new(snapshot);
        handle
            .update(&mut cx, |view, window, cx| {
                ui_scale::set_window_percent(cx, window.window_handle().window_id(), Some(percent));
                view.set_theme(
                    if name == "light" {
                        AppTheme::gitcomet_light()
                    } else {
                        AppTheme::gitcomet_dark()
                    },
                    cx,
                );
                view.apply_ui_scale_percent(percent, window, cx);
                test_support::push_test_state(view, snapshot, cx);
                test_support::set_sidebar_width_for_test(view, px(width), cx);
                view.sidebar_pane.update(cx, |pane, cx| {
                    let expanded = branch_sidebar::expanded_default_section_storage_key(
                        branch_sidebar::worktrees_section_storage_key(),
                    )
                    .unwrap();
                    pane.set_collapsed_keys_for_test(&[expanded.as_str()]);
                    cx.notify();
                });
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, app| {
            window.refresh();
            let _ = window.draw(app);
        })
        .unwrap();
        cx.capture_screenshot(handle.into())
            .unwrap()
            .save(directory.join(format!("{name}.png")))
            .unwrap();
    }
    let mut measurements = Vec::new();
    for shared in [true, false] {
        let fixture = crate::view::rows::benchmarks::BranchSidebarFixture::twenty_thousand_local_branches_five_thousand_worktrees(shared);
        let mut repo = fixture.repo;
        repo.id = RepoId(if shared { 2 } else { 3 });
        let repo_id = repo.id;
        let branches = Arc::make_mut(match &mut repo.branches {
            Loadable::Ready(branches) => branches,
            _ => unreachable!(),
        });
        for (index, branch) in branches.iter_mut().enumerate() {
            branch.name = if index == 0 {
                "000-active".into()
            } else {
                format!("branch-{index:05}")
            };
        }
        repo.head_branch = Loadable::Ready("000-active".into());
        if let Loadable::Ready(worktrees) = &mut repo.worktrees {
            for (index, worktree) in Arc::make_mut(worktrees).iter_mut().enumerate() {
                worktree.branch =
                    (!worktree.detached).then(|| branches[index % branches.len()].name.clone());
            }
        }
        let target = branches[0].target.clone();
        let mut projections = ComparisonProjections::default();
        for branch in branches.iter() {
            projections
                .branches
                .insert(branch.name.clone(), branch.target.clone());
        }
        let mut seen = rustc_hash::FxHashSet::default();
        projections.subjects = branches
            .iter()
            .filter_map(|branch| {
                seen.insert(branch.target.clone())
                    .then_some(branch.target.clone())
            })
            .collect();
        if let Loadable::Ready(worktrees) = &repo.worktrees {
            for worktree in worktrees.iter() {
                if let Some(head) = &worktree.head {
                    projections
                        .worktrees
                        .insert(worktree.path.clone(), head.clone());
                }
            }
        }
        repo.ref_comparisons.projections = Arc::new(projections);
        repo.ref_comparisons.target = Loadable::Ready(RefComparisonTarget {
            reference: "origin/dev".into(),
            oid: target,
            context: 1,
        });
        let mut snapshot = Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(repo_id),
            ..AppState::test_default()
        });
        let memory = Arc::new(ComparisonMemory::default());
        handle
            .update(&mut cx, |view, window, cx| {
                ui_scale::set_window_percent(cx, window.window_handle().window_id(), Some(100));
                view.apply_ui_scale_percent(100, window, cx);
                test_support::set_sidebar_width_for_test(view, px(500.0), cx);
                test_support::push_test_state(view, snapshot.clone(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(&mut cx, |view, _, cx| {
                view.sidebar_pane.update(cx, |pane, cx| {
                    let expanded = branch_sidebar::expanded_default_section_storage_key(
                        branch_sidebar::worktrees_section_storage_key(),
                    )
                    .unwrap();
                    pane.set_collapsed_keys_for_test(&[expanded.as_str()]);
                    cx.notify();
                });
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, app| {
            window.refresh();
            let _ = window.draw(app);
        })
        .unwrap();
        let rows = handle
            .update(&mut cx, |view, _, cx| {
                view.sidebar_pane.update(cx, |pane, _| {
                    pane.branch_sidebar_presentation_cached().unwrap().rows
                })
            })
            .unwrap();
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(
                    row,
                    branch_sidebar::BranchSidebarRow::Branch {
                        section: branch_sidebar::BranchSection::Local,
                        ..
                    }
                ))
                .count(),
            20_000
        );
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(row, branch_sidebar::BranchSidebarRow::WorktreeItem { .. }))
                .count(),
            5_000
        );
        assert!(rows.iter().any(|row| matches!(row, branch_sidebar::BranchSidebarRow::Branch { name, is_head: true, .. } if name.as_ref() == "000-active")));
        let subjects = snapshot.repos[0]
            .ref_comparisons
            .projections
            .subjects
            .clone();
        let mut frames = Vec::new();
        let mut publications = Vec::new();
        for (batch_index, batch) in subjects.chunks(128).take(20).enumerate() {
            let mut next = snapshot.as_ref().clone();
            let results = batch
                .iter()
                .enumerate()
                .map(|(index, subject)| RefComparison {
                    subject: subject.clone(),
                    counts: RefComparisonCounts::Exact {
                        ahead: 0,
                        behind: (batch_index * 128 + index) as u64,
                    },
                    integration: RefIntegration::Confirmed(if subject == &subjects[0] {
                        IntegrationEvidence::UpToDate
                    } else {
                        IntegrationEvidence::Merged
                    }),
                })
                .collect();
            let publication = std::time::Instant::now();
            next.repos[0]
                .ref_comparisons
                .decorations
                .publish(&memory, results)
                .unwrap();
            next.repos[0].ref_comparisons.rev += 1;
            publications.push(publication.elapsed().as_secs_f64() * 1000.0);
            snapshot = Arc::new(next);
            let frame = std::time::Instant::now();
            handle
                .update(&mut cx, |view, _, cx| {
                    test_support::push_test_state(view, snapshot.clone(), cx)
                })
                .unwrap();
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, app| {
                window.refresh();
                let _ = window.draw(app);
            })
            .unwrap();
            frames.push(frame.elapsed().as_secs_f64() * 1000.0);
            let updated = handle
                .update(&mut cx, |view, _, cx| {
                    view.sidebar_pane.update(cx, |pane, _| {
                        pane.branch_sidebar_presentation_cached().unwrap().rows
                    })
                })
                .unwrap();
            assert!(
                std::rc::Rc::ptr_eq(&rows, &updated),
                "comparison publications rebuilt structural rows"
            );
        }
        measurements.push(serde_json::json!({ "shared_tips": shared, "local_branches": 20_000, "worktrees": 5_000, "structural_rows": rows.len(), "visible_rows_cache_reused": true, "decoration_publication_elapsed_ms": publications, "apply_layout_paint_submit_ms": frames, "decoration_reserved_peak_bytes": memory.peak_bytes() }));
        cx.capture_screenshot(handle.into())
            .unwrap()
            .save(directory.join(if shared {
                "scale-shared.png"
            } else {
                "scale-distinct.png"
            }))
            .unwrap();
    }
    let report = serde_json::json!({ "date": "2026-10-25", "profile": "test; native fonts and headless GPU (record Cargo settings with the capture)", "scope": "Elapsed snapshot apply/layout/paint/submission including harness pumping; excludes screenshot readback and backend work", "release_frame_gate": "not established by this test-profile capture", "cases": measurements });
    std::fs::write(
        directory.join("frames.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "comparison previews and frame measurements: {}",
        directory.display()
    );
}
