# Mermaid regression corpus
Basic and focused .mmd cases are from mermaid-rs-renderer 0.3.1 (MIT),
https://github.com/1jehuang/mermaid-rs-renderer. See LICENSE.upstream.
Styled and invalid directive variants were added by GitComet. Each family
checks visible labels, finite geometry, a visible raster, and explicit errors.
Flowchart and sequence fixtures also cover shapes, edge routing, subgraphs,
cycles, styles and nested frames. These are native-language regression cases,
not a claim of complete mermaid.js compatibility.

`flowchart/supplied.mmd` is copied from
`diagrams/mermaid/diagrams.mmd` in
https://github.com/Havunen/syntax_highlight_test at commit
`d07d43b5a0ad982c1f5e49c979ed561bc0923a45` (MIT; see LICENSE.syntax_highlight_test).
It is kept in this crate so the rendering regression runs in clean checkouts
without initializing the optional `fixtures/syntax_test` submodule.
