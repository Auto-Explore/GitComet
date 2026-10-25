# GitComet Mermaid renderer patches

Upstream: mermaid-rs-renderer 0.3.1, MIT, https://github.com/1jehuang/mermaid-rs-renderer.
The checked-in source is the published crate, with these focused changes:

* Expose the parser's header recognizer. Require an explicit flowchart direction
  for inferred Markdown fences; edge-only ordinary code stays code.
* Extract the CLI's pure source configuration merger into source_config.rs, so
  the application can apply init directives without enabling the CLI. Publish
  the leaf options it implements for unsupported-setting diagnostics.
* Measure text using gitcomet-fonts. Remove system font discovery and the disk
  font cache. Cache up to 2048 exact text widths with bounded key lengths. Fall back to the same bundled sans family used by SVG decoding.

GitComet uses strict parsing, then configuration, layout and SVG exactly once.
Font selection, YAML frontmatter and SVG checks live in gitcomet-diagrams.
The upstream convenience APIs are retained for compatibility; GitComet does
not use them. Corpus attribution and regression expectations live alongside
GitComet's tests.

* Fix requirement labels to place their baseline below the top padding. Header
  and first body lines previously crossed the box border and divider.
