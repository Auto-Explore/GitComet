use super::*;

pub(super) fn file_diff_text_is_source_backed(file: &gitcomet_core::domain::FileDiffText) -> bool {
    file.old_source.is_some() || file.new_source.is_some()
}

pub(super) fn file_diff_markdown_source_len(
    source: Option<&gitcomet_core::domain::FileDiffTextSource>,
    legacy_text: Option<&Arc<str>>,
) -> usize {
    if let Some(text) = legacy_text {
        return text.len();
    }
    source
        .and_then(|source| std::fs::metadata(&source.path).ok())
        .and_then(|metadata| usize::try_from(metadata.len()).ok())
        .unwrap_or(0)
}

pub(super) fn read_file_diff_markdown_source(
    source: Option<&gitcomet_core::domain::FileDiffTextSource>,
    legacy_text: Option<&Arc<str>>,
) -> std::result::Result<String, String> {
    if let Some(text) = legacy_text {
        return Ok(text.to_string());
    }
    let Some(source) = source else {
        return Ok(String::new());
    };
    std::fs::read_to_string(&source.path).map_err(|err| err.to_string())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FileDiffPreparedSyntaxApplyResult {
    pub(super) split_left: bool,
    pub(super) split_right: bool,
}

impl FileDiffPreparedSyntaxApplyResult {
    pub(super) fn any(self) -> bool {
        self.split_left || self.split_right
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SyncFileDiffPreparedSyntaxApplyResult {
    pub(super) inserted: bool,
    pub(super) needs_background_prepare: bool,
}

#[cfg(test)]
pub(super) fn preview_lines_source_len(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| line.len())
        .sum::<usize>()
        .saturating_add(lines.len().saturating_sub(1))
}

pub(super) fn build_single_markdown_preview_document(
    source: &str,
) -> Result<Arc<markdown_preview::MarkdownPreviewDocument>, markdown_preview::MarkdownPreviewRefusal>
{
    use markdown_preview::MarkdownPreviewRefusal;

    if source.len() > markdown_preview::MAX_PREVIEW_SOURCE_BYTES {
        return Err(MarkdownPreviewRefusal::Unavailable(
            markdown_preview::single_preview_unavailable_reason(source.len()).to_owned(),
        ));
    }

    let document = markdown_preview::parse_markdown(source).ok_or_else(|| {
        MarkdownPreviewRefusal::Unavailable(
            markdown_preview::single_preview_unavailable_reason(source.len()).to_owned(),
        )
    })?;
    // The single-document preview lays every row out on every frame, so its
    // budget is tighter than the parser's. This one is recoverable: the source
    // is still readable, so the reader is sent there instead of to an error.
    if document.rows.len() > markdown_preview::MAX_FLOWING_PREVIEW_ROWS {
        return Err(MarkdownPreviewRefusal::TooManyRowsToRender);
    }

    Ok(Arc::new(document))
}

/// The pixel size of every picture in `document` that can be measured without
/// decoding it, keyed by the source the document wrote.
///
/// A picture's box is not known until its file has been read, and reading a GIF
/// means decoding every frame — seconds of work for a long one. Its header says
/// how big it is in a few bytes, which is enough to hold the right amount of
/// room open in the meantime. Runs beside the parse, off the UI thread, so a
/// document that carries no pictures pays nothing.
pub(super) fn measure_markdown_preview_pictures(
    document: &markdown_preview::MarkdownPreviewDocument,
    image_base_dir: Option<&std::path::Path>,
) -> rows::MarkdownPreviewPictureSizes {
    let mut sizes: FxHashMap<SharedString, (u32, u32)> = FxHashMap::default();
    let mut measure = |source: &SharedString| {
        if sizes.contains_key(source) {
            return;
        }
        // Only a local file can be measured this cheaply. A remote picture
        // would have to be fetched, which is the expensive half anyway, and
        // `gpui` is already fetching it.
        let Some(rows::MarkdownPreviewImageSource::File(path)) =
            rows::markdown_preview_image_source(image_base_dir, source.as_ref())
        else {
            return;
        };
        if let Ok((width, height)) = image::image_dimensions(&path)
            && width > 0
            && height > 0
        {
            sizes.insert(source.clone(), (width, height));
        }
    };

    for row in document.rows.iter() {
        if let Some(image) = row.image.as_ref() {
            measure(&image.source);
        }
        for inline in row.inline_images.iter() {
            measure(&inline.image.source);
        }
    }

    Arc::new(sizes)
}
