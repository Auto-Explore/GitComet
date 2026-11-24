use super::*;
use crate::view::image_preview::{Group, ImageViewport, Input, Source, raster_format};

pub(super) fn decode_image(path: &Path, bytes: &[u8]) -> Result<Input, String> {
    decode_image_with_limit(path, bytes, 512 * 1024 * 1024)
}

fn decode_image_with_limit(path: &Path, bytes: &[u8], detail_limit: u64) -> Result<Input, String> {
    let guessed = image::guess_format(bytes).ok();
    let format = match guessed {
        Some(_) => crate::view::diff_utils::image_format_for_bytes(bytes),
        None => image_format_for_path(path),
    };
    let Some((format, source)) = format
        .and_then(|format| Source::new(format, bytes, detail_limit).map(|source| (format, source)))
    else {
        // Match gpui::img's format detection and validate the header without
        // allocating the full raster. The document can use its legacy renderer
        // when detail decoding is over budget or needs another supported codec.
        let decoder = |format| {
            image::ImageReader::with_format(std::io::Cursor::new(bytes), format).into_decoder()
        };
        if let Some(format) = guessed {
            let decoder = decoder(format).map_err(|error| error.to_string())?;
            use image::ImageDecoder as _;
            if decoder.dimensions().0 == 0 || decoder.dimensions().1 == 0 {
                return Err("Empty image".into());
            }
        } else if let Err(error) = crate::view::image_preview::svg::parse(bytes) {
            // gpui::img renders unrecognized bytes as SVG, but a broken raster
            // file reports its own codec's error rather than an SVG one.
            return Err(match format.and_then(raster_format).map(decoder) {
                Some(Err(raster)) => raster.to_string(),
                _ => error.to_string(),
            });
        }
        return Ok(Input::default());
    };
    let preview = match source.svg() {
        Some(tree) => panes::diff::diff_cache::render_svg_image_diff_preview(tree),
        None => panes::diff::diff_cache::render_raster_image_diff_preview(format, bytes),
    };
    Ok(Input {
        source: Some(source),
        preview,
        frame: 0,
    })
}

impl StandaloneBuffer {
    pub(super) fn image_ready(&self) -> bool {
        self.image && !self.loading() && self.error().is_none()
    }

    pub(super) fn render_image(
        &mut self,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        if self.image_input.source.is_none() {
            if let Some(group) = self.image_group.as_ref() {
                group.update(cx, |group, cx| {
                    group.set_inputs(
                        self.identity().0.to_string_lossy().into_owned(),
                        Default::default(),
                        false,
                        cx,
                    )
                });
            }
            let theme = self.theme;
            return (
                gpui::img(self.identity().0.clone())
                    .id("document_image_fallback")
                    .debug_selector(|| "document_image_fallback".into())
                    .size_full()
                    .object_fit(gpui::ObjectFit::Contain)
                    .with_fallback(move || {
                        components::empty_state_message(theme, "Preview unavailable.")
                            .into_any_element()
                    })
                    .into_any_element(),
                None,
            );
        }
        let group = if let Some(group) = self.image_group.as_ref() {
            group.clone()
        } else {
            // Documents share the main pane's tiles; one of them shows at a time.
            let owner = self.root.upgrade().map(|root| {
                root.read(cx)
                    .diff_renderer
                    .read(cx)
                    .image_preview_group
                    .clone()
            });
            let group = cx.new(|cx| match owner {
                Some(owner) => Group::shared(&owner, cx),
                None => Group::new(cx),
            });
            cx.observe(&group, |_, _, cx| cx.notify()).detach();
            self.image_group = Some(group.clone());
            group
        };
        let paused = group.read(cx).paused;
        let deadline = if paused {
            self.image_animation.frame_started_at = None;
            None
        } else {
            DiffRenderer::advance_file_image_preview_side(
                &mut self.image_animation,
                self.image_input.preview.as_ref(),
                std::time::Instant::now(),
                crate::view::image_preview::animates(window, cx),
            )
        };
        self.image_timer
            .schedule(deadline, |this: &mut Self| &mut this.image_timer, cx);
        self.image_input.frame = self.image_animation.frame_index;
        let target = self.identity().0.to_string_lossy().into_owned();
        let cap = preview_path_uses_scale_down(&self.identity().0);
        group.update(cx, |group, cx| {
            group.set_inputs(
                target,
                [self.image_input.clone(), Input::default(), Input::default()],
                cap,
                cx,
            )
        });
        let controls = crate::view::image_preview::toolbar(
            group.clone(),
            self.theme,
            cx,
            |this, menu, invoker, position, window, cx| {
                let request = crate::view::panes::diff::image_menu_request(menu, invoker);
                let _ = this.root.update(cx, |root, cx| {
                    root.open_popover_at(request, position, window, cx)
                });
            },
        );
        (
            ImageViewport {
                group,
                side: 0,
                theme: self.theme,
            }
            .into_any_element(),
            Some(controls),
        )
    }
}

#[cfg(test)]
mod tests;
