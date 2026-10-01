//! Rendered conflict previews: SVG and raster images.

use super::*;

#[gpui::test]
fn svg_conflict_preview_rasterizes_off_the_ui_thread(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(163);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_conflict_preview",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_preview.svg");
    let abs_path = workdir.join(&file_rel);

    let base_svg: Arc<str> =
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" fill="#1d4ed8"/></svg>"##
            .into();
    let ours_svg: Arc<str> =
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="7" fill="#dc2626"/></svg>"##
            .into();
    let theirs_svg: Arc<str> =
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M2 14L8 2L14 14Z" fill="#16a34a"/></svg>"##
            .into();
    let current_text: Arc<str> = format!(
        "<<<<<<< ours\n{}\n=======\n{}\n>>>>>>> theirs\n",
        ours_svg, theirs_svg
    )
    .into();

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("svg conflict preview parent"))
        .expect("create svg conflict preview dir");
    std::fs::write(&abs_path, current_text.as_bytes()).expect("write svg conflict preview");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.conflict_state.conflict_file_path = Some(file_rel.clone());
            repo.conflict_state.conflict_file =
                gitcomet_state::model::Loadable::Ready(Some(gitcomet_state::model::ConflictFile {
                    path: file_rel.clone().into(),
                    base_bytes: None,
                    ours_bytes: None,
                    theirs_bytes: None,
                    current_bytes: None,
                    base: Some(base_svg.clone()),
                    ours: Some(ours_svg.clone()),
                    theirs: Some(theirs_svg.clone()),
                    current: Some(current_text.clone()),
                }));
            repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
                file_rel.clone(),
                gitcomet_core::domain::FileConflictKind::BothModified,
                ConflictPayload::Text(base_svg.clone()),
                ConflictPayload::Text(ours_svg.clone()),
                ConflictPayload::Text(theirs_svg.clone()),
                &current_text,
            ));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "svg conflict resolver bootstrap initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.source_hash.is_some()
        },
        |pane| {
            format!(
                "path={:?} source_hash={:?} preview_path={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.source_hash,
                pane.conflict_resolver.image_preview.path.clone(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_conflict_image_preview_cache(cx);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "svg conflict preview cache rasterized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            matches!(
                pane.conflict_resolver.image_preview.image(ThreeWayColumn::Base),
                Loadable::Ready(Some(ConflictPreviewImage::Encoded(image)))
                    if image.format() == gpui::ImageFormat::Png
            ) && matches!(
                pane.conflict_resolver.image_preview.image(ThreeWayColumn::Ours),
                Loadable::Ready(Some(ConflictPreviewImage::Encoded(image)))
                    if image.format() == gpui::ImageFormat::Png
            ) && matches!(
                pane.conflict_resolver.image_preview.image(ThreeWayColumn::Theirs),
                Loadable::Ready(Some(ConflictPreviewImage::Encoded(image)))
                    if image.format() == gpui::ImageFormat::Png
            )
        },
        |pane| {
            format!(
                "base={:?} ours={:?} theirs={:?}",
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Base),
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Ours),
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Theirs),
            )
        },
    );

    // The resolved output is now materialized into the editable buffer at
    // bootstrap (kdiff3-style free-text editing), which recomputes the output
    // outline/syntax and can leave background work pending. Drain it so no stray
    // task remains when the deterministic test scheduler ends.
    cx.run_until_parked();

    std::fs::remove_dir_all(&workdir).expect("cleanup svg conflict preview fixture");
}

#[gpui::test]
fn raster_conflict_preview_uses_alpha_correct_oriented_render_images(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    fn transparent_edge_png(rgb: [u8; 3]) -> Arc<[u8]> {
        use image::ImageEncoder as _;

        let rotate_90_exif = vec![
            b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0,
            0,
        ];
        let mut encoded = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut encoded);
        encoder
            .set_exif_metadata(rotate_90_exif)
            .expect("set PNG orientation");
        encoder
            .write_image(
                &[rgb[0], rgb[1], rgb[2], 255, 0, 0, 0, 0],
                2,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .expect("encode png fixture");
        Arc::from(encoded)
    }

    fn has_extended_bgra_edge(image: &LoadableImagePreview, expected: [u8; 4]) -> bool {
        let Loadable::Ready(Some(ConflictPreviewImage::Rendered(image))) = image else {
            return false;
        };
        let size = image.size(0);
        (size.width.0, size.height.0) == (1, 2)
            && image
                .as_bytes(0)
                .and_then(|bytes| bytes.get(4..8))
                .is_some_and(|pixel| pixel == expected)
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(164);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_raster_conflict_preview",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_preview.png");
    let abs_path = workdir.join(&file_rel);
    let base = transparent_edge_png([10, 20, 30]);
    let ours = transparent_edge_png([40, 50, 60]);
    let theirs = transparent_edge_png([70, 80, 90]);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("png conflict preview parent"))
        .expect("create png conflict preview dir");
    std::fs::write(&abs_path, ours.as_ref()).expect("write png conflict preview");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.conflict_state.conflict_file_path = Some(file_rel.clone());
            repo.conflict_state.conflict_file =
                gitcomet_state::model::Loadable::Ready(Some(gitcomet_state::model::ConflictFile {
                    path: file_rel.clone().into(),
                    base_bytes: Some(Arc::clone(&base)),
                    ours_bytes: Some(Arc::clone(&ours)),
                    theirs_bytes: Some(Arc::clone(&theirs)),
                    current_bytes: Some(Arc::clone(&ours)),
                    base: None,
                    ours: None,
                    theirs: None,
                    current: None,
                }));
            repo.conflict_state.conflict_session = Some(ConflictSession::new_with_current(
                file_rel.clone(),
                gitcomet_core::domain::FileConflictKind::BothModified,
                ConflictPayload::Binary(Arc::clone(&base)),
                ConflictPayload::Binary(Arc::clone(&ours)),
                ConflictPayload::Binary(Arc::clone(&theirs)),
                ConflictPayload::Binary(Arc::clone(&ours)),
            ));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "raster conflict resolver bootstrap initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.source_hash.is_some()
        },
        |pane| {
            format!(
                "path={:?} source_hash={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.source_hash,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_conflict_image_preview_cache(cx);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "raster conflict preview cache decoded",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            has_extended_bgra_edge(
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Base),
                [30, 20, 10, 0],
            ) && has_extended_bgra_edge(
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Ours),
                [60, 50, 40, 0],
            ) && has_extended_bgra_edge(
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Theirs),
                [90, 80, 70, 0],
            )
        },
        |pane| {
            format!(
                "base={:?} ours={:?} theirs={:?}",
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Base),
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Ours),
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Theirs),
            )
        },
    );

    cx.run_until_parked();
    std::fs::remove_dir_all(&workdir).expect("cleanup raster conflict preview fixture");
}
