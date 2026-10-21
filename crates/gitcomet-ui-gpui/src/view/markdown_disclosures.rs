use super::*;
use crate::view::markdown_preview::{
    MarkdownPreviewDocument, MarkdownPreviewRow, MarkdownPreviewRowKind,
};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Default)]
pub(in crate::view) struct MarkdownDisclosures(Rc<RefCell<DisclosureState>>);

#[derive(Default)]
struct DisclosureState {
    source: Option<Arc<MarkdownPreviewDocument>>,
    expanded: FxHashMap<usize, bool>,
    projected: Option<Arc<MarkdownPreviewDocument>>,
    click: crate::kit::click::SubtargetClick<usize>,
}

impl MarkdownDisclosures {
    pub(in crate::view) fn is_summary(&self, row: usize) -> bool {
        self.0
            .borrow()
            .source
            .as_ref()
            .and_then(|document| document.rows.get(row))
            .is_some_and(|row| row.kind == MarkdownPreviewRowKind::DetailsSummary)
    }

    pub(in crate::view) fn begin_click(&self, id: usize, event: &gpui::MouseDownEvent) {
        self.0.borrow_mut().click.press(Some(id), event);
    }

    pub(in crate::view) fn pointer_moved(&self, event: &gpui::MouseMoveEvent) {
        self.0.borrow_mut().click.moved(event);
    }

    pub(in crate::view) fn cancel_click(&self) {
        self.0.borrow_mut().click = Default::default();
    }

    pub(in crate::view) fn complete_click(&self, id: usize, event: &gpui::MouseUpEvent) -> bool {
        self.0
            .borrow_mut()
            .click
            .release(Some(&id), event)
            .is_some()
    }

    pub(in crate::view) fn project(
        &self,
        document: &Arc<MarkdownPreviewDocument>,
    ) -> Arc<MarkdownPreviewDocument> {
        let mut state = self.0.borrow_mut();
        if state
            .source
            .as_ref()
            .is_none_or(|source| !Arc::ptr_eq(source, document))
        {
            *state = DisclosureState {
                source: Some(document.clone()),
                ..Default::default()
            };
        }
        if let Some(projected) = &state.projected {
            return projected.clone();
        }
        let projected = if document.rows.iter().all(|row| state.is_visible(row)) {
            document.clone()
        } else {
            Arc::new(MarkdownPreviewDocument::new(
                document
                    .rows
                    .iter()
                    .map(|row| {
                        if state.is_visible(row) {
                            row.clone()
                        } else {
                            MarkdownPreviewRow::default()
                        }
                    })
                    .collect(),
            ))
        };
        state.projected = Some(projected.clone());
        projected
    }

    pub(in crate::view) fn is_visible(&self, row: &MarkdownPreviewRow) -> bool {
        self.0.borrow().is_visible(row)
    }

    pub(in crate::view) fn would_hide(&self, row: &MarkdownPreviewRow, id: usize) -> bool {
        self.is_visible(row) && enclosing_details(row).any(|section| section.id == id)
    }

    pub(in crate::view) fn is_open(
        &self,
        details: &crate::view::markdown_preview::MarkdownDetails,
    ) -> bool {
        self.0
            .borrow()
            .expanded
            .get(&details.id)
            .copied()
            .unwrap_or(details.initially_open)
    }

    pub(in crate::view) fn toggle(&self, id: usize) -> bool {
        let mut state = self.0.borrow_mut();
        let details = state.source.as_ref().and_then(|source| {
            source.rows.iter().find_map(|row| {
                row.details
                    .as_ref()
                    .filter(|details| details.id == id)
                    .cloned()
            })
        });
        let Some(details) = details else {
            return false;
        };
        let open = state
            .expanded
            .get(&id)
            .copied()
            .unwrap_or(details.initially_open);
        state.expanded.insert(id, !open);
        state.projected = None;
        true
    }

    pub(in crate::view) fn reveal(&self, row: &MarkdownPreviewRow) -> bool {
        let mut state = self.0.borrow_mut();
        let mut changed = false;
        for section in enclosing_details(row) {
            let open = state
                .expanded
                .get(&section.id)
                .copied()
                .unwrap_or(section.initially_open);
            if !open {
                state.expanded.insert(section.id, true);
                changed = true;
            }
        }
        if changed {
            state.projected = None;
        }
        changed
    }
}

impl DisclosureState {
    fn is_visible(&self, row: &MarkdownPreviewRow) -> bool {
        enclosing_details(row).all(|section| {
            self.expanded
                .get(&section.id)
                .copied()
                .unwrap_or(section.initially_open)
        })
    }
}

fn enclosing_details(
    row: &MarkdownPreviewRow,
) -> impl Iterator<Item = &crate::view::markdown_preview::MarkdownDetails> {
    let first = if row.kind == MarkdownPreviewRowKind::DetailsSummary {
        row.details
            .as_deref()
            .and_then(|details| details.parent.as_deref())
    } else {
        row.details.as_deref()
    };
    std::iter::successors(first, |section| section.parent.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::markdown_preview::{markdown_document_blocks, parse_markdown};

    #[test]
    fn disclosure_projection_preserves_canonical_rows_and_invalidates_equal_size_swaps() {
        let source = Arc::new(parse_markdown(
            "<details><summary>First</summary>\n\nFirst body\n\n</details>\n\n<details><summary>Second</summary>\n\nSecond body\n\n</details>",
        ).unwrap());
        let original = (*source).clone();
        let state = MarkdownDisclosures::default();
        let summaries: Vec<_> = source
            .rows
            .iter()
            .filter(|row| row.kind == MarkdownPreviewRowKind::DetailsSummary)
            .map(|row| row.details.as_ref().unwrap().id)
            .collect();
        let initial = state.project(&source);
        assert_eq!(markdown_document_blocks(&initial).len(), 2);
        assert!(state.toggle(summaries[0]));
        let first = state.project(&source);
        assert!(state.toggle(summaries[0]));
        assert!(state.toggle(summaries[1]));
        let second = state.project(&source);
        assert_eq!(
            markdown_document_blocks(&first).len(),
            markdown_document_blocks(&second).len()
        );
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(source.as_ref(), &original);
        let first_body = source
            .rows
            .iter()
            .position(|row| row.text.as_ref() == "First body")
            .unwrap();
        let second_body = source
            .rows
            .iter()
            .position(|row| row.text.as_ref() == "Second body")
            .unwrap();
        assert_eq!(first.rows[first_body].text.as_ref(), "First body");
        assert!(second.rows[first_body].is_alignment_padding());
        assert_eq!(second.rows[second_body].text.as_ref(), "Second body");
        assert_eq!(
            second.rows[second_body].source_line_range,
            source.rows[second_body].source_line_range
        );
        assert!(Arc::ptr_eq(&second, &state.project(&source)));
    }

    #[test]
    fn reveal_expands_ancestors_and_replacement_source_resets_disclosures() {
        let source = Arc::new(parse_markdown(
            "<details><summary>Outer</summary>\n\n<details><summary>Inner</summary>\n\n# Target\n\n</details>\n\n</details>",
        ).unwrap());
        let state = MarkdownDisclosures::default();
        state.project(&source);
        let target = source
            .rows
            .iter()
            .find(|row| row.text.as_ref() == "Target")
            .unwrap();
        assert!(!state.is_visible(target));
        assert!(state.reveal(target));
        assert!(state.is_visible(target));
        let replacement = Arc::new((*source).clone());
        state.project(&replacement);
        assert!(!state.is_visible(target));
    }
}
