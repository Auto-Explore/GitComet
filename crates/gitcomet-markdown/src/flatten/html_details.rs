use super::*;

pub(super) struct OpenDetails {
    metadata: Arc<MarkdownDetails>,
    summary_emitted: bool,
    source: Range<usize>,
    markdown_depth: usize,
    html_depth: usize,
    container_depth: usize,
    html_container_depth: usize,
    styles: usize,
    links: usize,
}

impl Flattener<'_> {
    pub(super) fn current_details(&self) -> Option<Arc<MarkdownDetails>> {
        self.details.last().map(|open| open.metadata.clone())
    }

    pub(super) fn open_details(
        &mut self,
        initially_open: bool,
        source: Range<usize>,
    ) -> Option<()> {
        u8::try_from(self.details.len() + 1).ok()?;
        self.split_row_at(source.start)?;
        self.ensure_details_summary()?;
        self.close_html_paragraphs();
        self.details.push(OpenDetails {
            metadata: Arc::new(MarkdownDetails {
                id: source.start,
                initially_open,
                parent: self.current_details(),
            }),
            summary_emitted: false,
            source,
            markdown_depth: self.markdown_container_depth(),
            html_depth: self.html_container_depth(),
            container_depth: self.containers.len(),
            html_container_depth: self.html_containers.len(),
            styles: self.styles.len(),
            links: self.links.len(),
        });
        Some(())
    }

    pub(super) fn close_details(&mut self, at: usize) -> Option<()> {
        if self.details.is_empty() {
            return Some(());
        }
        self.split_row_at(at)?;
        self.ensure_details_summary()?;
        let open = self.details.pop()?;
        self.discard_details(open);
        Some(())
    }

    fn discard_details(&mut self, open: OpenDetails) {
        let mut index = 0;
        self.containers.retain(|container| {
            let retain =
                index < open.container_depth || container.owner == ContainerOwner::Markdown;
            index += 1;
            retain
        });
        self.html_containers.truncate(open.html_container_depth);
        self.styles.truncate(open.styles);
        self.links.truncate(open.links);
        self.html_links = self.html_links.min(self.links.len());
        self.clamp_html_floor();
    }

    pub(super) fn close_details_outside_owner_scope(&mut self) {
        let markdown = self.markdown_container_depth();
        let html = self.html_container_depth();
        while self
            .details
            .last()
            .is_some_and(|open| open.markdown_depth > markdown || open.html_depth > html)
        {
            if let Some(open) = self.details.pop() {
                self.discard_details(open);
            }
        }
    }

    pub(super) fn details_summary_kind(&mut self) -> MarkdownPreviewRowKind {
        if let Some(open) = self.details.last_mut()
            && std::mem::replace(&mut open.summary_emitted, true)
        {
            return MarkdownPreviewRowKind::Paragraph;
        }
        MarkdownPreviewRowKind::DetailsSummary
    }

    pub(super) fn ensure_details_summary(&mut self) -> Option<()> {
        let Some(open) = self.details.last_mut().filter(|open| !open.summary_emitted) else {
            return Some(());
        };
        open.summary_emitted = true;
        let source = open.source.clone();
        let row = MarkdownPreviewRowInput::plain(
            MarkdownPreviewRowKind::DetailsSummary,
            "Details",
            &[],
            self.line_range(source),
            self.indent_level(),
            self.blockquote_level(),
        );
        let decoration = MarkdownPreviewRowDecoration {
            details: self.current_details(),
            ordered_list_start: self.ordered_list_start(),
            align: self.html_align(),
            ..MarkdownPreviewRowDecoration::default()
        };
        push_row(&mut self.rows, row, decoration)
    }
}
