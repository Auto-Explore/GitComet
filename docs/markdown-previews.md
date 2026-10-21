# Markdown Previews

Document previews and extension descriptions share Markdown rendering, text selection, links, and HTML disclosures.

Common HTML formatting renders alongside Markdown: headings, paragraphs, emphasis, links, lists, quotes, preformatted code, tables, and images. Character references decode into their text. Unsupported HTML keeps the parser's inert fallback.

In document previews and embedded descriptions, `<details>` starts closed unless it has an `open` attribute. Click anywhere on its heading, or focus the heading and press Enter or Space, to toggle it. Dragging selects the summary text, and links within the summary keep their usual behavior. Nested sections retain their own state. Inline and split diff previews show every section expanded.

Drag to select text, double-click to select a word, or triple-click to select a row. Copy and Select All use Cmd-C and Cmd-A on macOS, or Ctrl-C and Ctrl-A elsewhere. Copy includes only visible content. Closing a section clears a selection that includes its hidden content. Searching a document or following a heading link opens the enclosing sections before revealing the target.

Click a link to open its menu; Cmd-click or Ctrl-click follows it directly. Dragging over link text selects it. Images obey the window's remote-image preference. Embedded descriptions have no local document path, so relative file links stay inert.

## Embedding a preview

`WindowHost::create_markdown_view` returns a content-height view. Place it inside the extension's scroll container, supply the source, and associate that container's `ScrollHandle`:

```rust
let preview = window_host.create_markdown_view(cx)?;
preview.set_source(description, cx);
preview.set_scroll_handle(Some(scroll.clone()), cx);
```

The shared scroll handle lets selection scroll at the container edges and lets heading links reveal their targets. Pass `None` to detach it. Setting identical source preserves selection and disclosure state; changing the source resets them.
