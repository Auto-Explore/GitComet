use super::*;

#[gpui::test]
fn file_image_diff_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(147);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_image_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/gitcomet.png");
    let image_bytes =
        include_bytes!("../../../../../../../../assets/linux/hicolor/32x32/apps/gitcomet.png")
            .to_vec();

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(image_bytes.as_slice()),
        Some(image_bytes.as_slice()),
    );
    wait_for_file_image_diff_cache(cx, &view, "initial image diff cache build", |_| true);

    let baseline_seq = cx.update(|_window, app| {
        view.read(app)
            .diff_renderer
            .read(app)
            .file_image_diff_cache_seq
    });

    for rev in 2..=6 {
        seed_file_image_diff_state_with_rev(
            cx,
            &view,
            repo_id,
            &workdir,
            &path,
            rev,
            Some(image_bytes.as_slice()),
            Some(image_bytes.as_slice()),
        );
        draw_and_drain_test_window(cx);

        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(
                pane.file_image_diff_cache_seq, baseline_seq,
                "identical image diff payload should not trigger cache rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_image_diff_cache_inflight.is_none(),
                "image diff cache should remain ready with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_image_diff_cache_rev, rev,
                "identical payload refresh should still advance the image diff cache rev marker"
            );
            assert!(
                pane.is_file_image_diff_view_active(),
                "image diff preview should remain active across rev-only refreshes"
            );
        });
    }
}

/// A name without an extension leaves the format to the bytes: a PNG named
/// `gitcomet` decodes and shows like `gitcomet.png`.
#[gpui::test]
fn file_image_diff_decodes_an_image_named_without_an_extension(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(148);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_image_diff_without_extension",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/gitcomet");
    let image_bytes =
        include_bytes!("../../../../../../../../assets/linux/hicolor/32x32/apps/gitcomet.png");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(image_bytes),
        Some(image_bytes),
    );
    wait_for_file_image_diff_cache(cx, &view, "both sides decoded", |pane| {
        pane.file_image_diff_cache_old.is_some() && pane.file_image_diff_cache_new.is_some()
    });
}

/// An untracked file previews as text, unless it is an image: once its bytes
/// have shown that a file without an extension is one, it shows as an image.
#[gpui::test]
fn an_untracked_image_named_without_an_extension_shows_as_an_image(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(149);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_untracked_image_without_extension",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("logo");
    let image_bytes =
        include_bytes!("../../../../../../../../assets/linux/hicolor/32x32/apps/gitcomet.png");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(workdir.join(&path), image_bytes).unwrap();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.content_image_target = repo.diff_state.diff_target.clone();
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file_image = gitcomet_state::model::Loadable::Ready(Some(
                Arc::new(gitcomet_core::domain::FileDiffImage {
                    path: path.clone(),
                    new: Some(image_bytes.to_vec()),
                    ..Default::default()
                }),
            ));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    wait_for_file_image_diff_cache(cx, &view, "the untracked image", |pane| {
        pane.file_image_diff_cache_new.is_some() && !pane.diff_surface().file_preview
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn replacing_file_image_diff_cache_releases_old_sprite_atlas_tiles(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(156);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_image_diff_atlas_release",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/gitcomet.png");
    let first_bytes =
        include_bytes!("../../../../../../../../assets/linux/hicolor/32x32/apps/gitcomet.png");
    let second_bytes =
        include_bytes!("../../../../../../../../assets/linux/hicolor/48x48/apps/gitcomet.png");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(first_bytes),
        Some(first_bytes),
    );
    wait_for_file_image_diff_cache(cx, &view, "initial atlas-backed image cache", |pane| {
        pane.file_image_diff_cache_old.is_some()
    });
    draw_and_drain_test_window(cx);

    let first_image = cx.update(|window, app| {
        let image = view
            .read(app)
            .diff_renderer
            .read(app)
            .file_image_diff_cache_old
            .as_ref()
            .expect("first render image")
            .clone();
        assert!(window.has_image_atlas_entry(&image));
        image
    });

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        2,
        Some(second_bytes),
        Some(second_bytes),
    );
    wait_for_file_image_diff_cache(cx, &view, "replacement atlas-backed image cache", |pane| {
        pane.file_image_diff_cache_old
            .as_ref()
            .is_some_and(|image| image.id != first_image.id)
    });
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        assert!(!window.has_image_atlas_entry(&first_image));
        let replacement = view
            .read(app)
            .diff_renderer
            .read(app)
            .file_image_diff_cache_old
            .as_ref()
            .expect("replacement render image");
        assert!(window.has_image_atlas_entry(replacement));
    });
}

#[gpui::test]
fn file_image_diff_cache_keeps_valid_svg_on_render_fast_path_across_rev_refreshes(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(148);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_image_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/diagram.svg");
    let svg_bytes = image_diff_svg_fixture(4096, 2048, "#00aaff");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(svg_bytes.as_slice()),
        Some(svg_bytes.as_slice()),
    );
    wait_for_file_image_diff_cache(cx, &view, "initial svg image diff cache build", |pane| {
        pane.file_image_diff_cache_old.is_some()
            && pane.file_image_diff_cache_new.is_some()
            && pane.file_image_diff_cache_old_svg_path.is_none()
            && pane.file_image_diff_cache_new_svg_path.is_none()
    });

    let baseline_seq = cx.update(|_window, app| {
        view.read(app)
            .diff_renderer
            .read(app)
            .file_image_diff_cache_seq
    });

    for rev in 2..=6 {
        seed_file_image_diff_state_with_rev(
            cx,
            &view,
            repo_id,
            &workdir,
            &path,
            rev,
            Some(svg_bytes.as_slice()),
            Some(svg_bytes.as_slice()),
        );
        draw_and_drain_test_window(cx);

        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(
                pane.file_image_diff_cache_seq, baseline_seq,
                "identical svg image diff payload should not trigger cache rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_image_diff_cache_inflight.is_none(),
                "svg image diff cache should remain ready with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_image_diff_cache_rev, rev,
                "identical svg payload refresh should still advance the image diff cache rev marker"
            );
            assert!(
                pane.file_image_diff_cache_old.is_some() && pane.file_image_diff_cache_new.is_some(),
                "valid svg payload should stay on the rasterized render-image path"
            );
            assert!(
                pane.file_image_diff_cache_old_svg_path.is_none()
                    && pane.file_image_diff_cache_new_svg_path.is_none(),
                "valid svg payload should not fall back to cached svg file paths"
            );
            assert!(
                pane.is_file_image_diff_view_active(),
                "svg image diff preview should remain active across rev-only refreshes"
            );
        });
    }
}

#[gpui::test]
fn file_image_diff_cache_keeps_distinct_valid_svg_sides_on_render_fast_path(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(149);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_image_diff_distinct",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/diagram.svg");
    let old_svg = image_diff_svg_fixture(4096, 2048, "#00aaff");
    let new_svg = image_diff_svg_fixture(2048, 4096, "#ffaa00");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(old_svg.as_slice()),
        Some(new_svg.as_slice()),
    );
    wait_for_file_image_diff_cache(
        cx,
        &view,
        "distinct svg image diff render cache build",
        |pane| {
            pane.file_image_diff_cache_old.is_some()
                && pane.file_image_diff_cache_new.is_some()
                && pane.file_image_diff_cache_old_svg_path.is_none()
                && pane.file_image_diff_cache_new_svg_path.is_none()
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        let old = pane
            .file_image_diff_cache_old
            .as_ref()
            .expect("old render image");
        let new = pane
            .file_image_diff_cache_new
            .as_ref()
            .expect("new render image");
        assert_eq!(old.size(0).width.0, 1024);
        assert_eq!(old.size(0).height.0, 512);
        assert_eq!(new.size(0).width.0, 512);
        assert_eq!(new.size(0).height.0, 1024);
    });
}

#[gpui::test]
fn file_image_diff_cache_falls_back_to_cached_svg_paths_for_invalid_svg_payloads(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(150);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_image_diff_invalid",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/diagram.svg");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(&b"<not-valid-svg-old>"[..]),
        Some(&b"<not-valid-svg-new>"[..]),
    );
    wait_for_file_image_diff_cache(
        cx,
        &view,
        "invalid svg image diff fallback cache build",
        |pane| {
            pane.file_image_diff_cache_old.is_none()
                && pane.file_image_diff_cache_new.is_none()
                && pane.file_image_diff_cache_old_svg_path.is_some()
                && pane.file_image_diff_cache_new_svg_path.is_some()
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.file_image_diff_cache_old_svg_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
        assert!(
            pane.file_image_diff_cache_new_svg_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
    });
}

#[gpui::test]
fn failed_raster_image_diff_cache_is_terminal_instead_of_processing_forever(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(155);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_failed_raster_image_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/broken.png");
    let invalid = b"not a png";

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(invalid),
        Some(invalid),
    );
    wait_for_file_image_diff_cache(cx, &view, "failed raster cache completion", |pane| {
        pane.file_image_diff_cache_complete
    });

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(pane.file_image_diff_cache_inflight.is_none());
        assert!(pane.file_image_diff_cache_failed);
        assert!(pane.is_file_image_diff_view_active());
    });
}

/// An untracked SVG is preview-only, so no patch is loaded for it — but an SVG
/// never reaches the text-file preview path, so the diff pane's Code view is
/// the only place its source is ever shown. It has to render the file text in
/// either diff mode, and the Image/Code toggle has to stay reachable.
#[gpui::test]
fn untracked_svg_keeps_the_code_view_and_toggle_in_collapsed_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(151);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_untracked_svg_code_view",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = PathBuf::from("assets/diagram.svg");
    let source = String::from_utf8(image_diff_svg_fixture(64, 64, "#22cc66"))
        .expect("svg fixture should be utf-8");
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        path.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = 1;
            // Preview-only: the state layer loads no patch for an untracked file.
            repo.diff_state.diff = gitcomet_state::model::Loadable::NotLoaded;
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(path.clone(), None, Some(source.clone())),
            )));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);

            this.diff_renderer.update(cx, |pane, _cx| {
                pane.diff_content_mode = DiffContentMode::Collapsed;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Svg, RenderedPreviewMode::Source);
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            !pane.is_file_preview_active(),
            "an SVG is classified as an image, so it must not take the text-file preview path"
        );
        // Nothing to collapse without a patch, so the pane falls back to Full.
        assert_eq!(pane.effective_diff_content_mode(), DiffContentMode::Full);
        assert!(pane.wants_file_diff_view(false));
        assert!(!pane.is_collapsed_diff_projection_active());
        assert_eq!(
            crate::view::main_diff_rendered_preview_toggle_kind(
                pane.wants_file_diff_view(false),
                pane.wants_collapsed_diff_view(false),
                false,
                crate::view::diff_target_rendered_preview_kind(Some(&target)),
            ),
            Some(RenderedPreviewKind::Svg),
            "the Image/Code toggle must stay available while Collapsed is selected"
        );
        assert!(
            pane.file_diff_inline_row_len() > 0,
            "the Code view should have the SVG source to render"
        );
    });
}

#[gpui::test]
fn image_preview_controls_and_wheel_take_precedence_over_window_zoom(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let dir = tempfile::tempdir().unwrap();
    let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        gitcomet_state::model::RepoId(199),
        dir.path(),
        std::path::Path::new("image.png"),
        1,
        Some(bytes.get_ref()),
        Some(bytes.get_ref()),
    );
    wait_for_file_image_diff_cache(cx, &view, "image inspection cache", |pane| {
        pane.file_image_diff_cache_complete
    });
    draw_and_drain_test_window(cx);
    assert!(cx.debug_bounds("image_format_strip").is_some());
    let bounds = cx.debug_bounds("image_viewport").expect("image viewport");
    let before = cx.update(|_, app| {
        let pane = view.read(app).diff_renderer.clone();
        pane.update(app, |pane, cx| {
            pane.image_zoom(crate::view::image_preview::Zoom::Scale(1.0), cx)
        });
        crate::ui_scale::current(app).percent
    });
    draw_and_drain_test_window(cx);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(60.0))),
        modifiers: gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|_, app| {
        assert_eq!(crate::ui_scale::current(app).percent, before);
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.image_preview_group.read(app).zoom_label(), "125%");
    });
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(px(5.0), px(5.0)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(60.0))),
        modifiers: gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|_, app| assert!(crate::ui_scale::current(app).percent > before));
}

#[gpui::test]
fn image_preview_lifecycle_invalid_svg_path_keeps_the_unavailable_placeholder(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let dir = tempfile::tempdir().unwrap();
    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        gitcomet_state::model::RepoId(200),
        dir.path(),
        std::path::Path::new("invalid.svg"),
        1,
        None,
        Some(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><broken"),
    );
    wait_for_file_image_diff_cache(cx, &view, "invalid SVG cache", |pane| {
        pane.file_image_diff_cache_complete
    });
    cx.update(|_, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.file_image_diff_cache_new_svg_path.is_some(),
            "exercise the legacy SVG path renderer"
        );
        assert!(!pane.file_image_diff_cache_failed);
    });
    let deadline = std::time::Instant::now() + DEFAULT_MAIN_PANE_WAIT_TIMEOUT;
    loop {
        draw_and_drain_test_window(cx);
        if cx.debug_bounds("image_preview_unavailable").is_some() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "a failed SVG asset must show Preview unavailable rather than an empty pane"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
