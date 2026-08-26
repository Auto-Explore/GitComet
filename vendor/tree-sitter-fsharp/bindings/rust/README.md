# tree-sitter-fsharp

F# grammars for the [tree-sitter][] parsing library, packaged as a Rust crate.

The crate ships **two** parsers:

| Constant             | Grammar              | File types    |
| -------------------- | -------------------- | ------------- |
| `LANGUAGE_FSHARP`    | implementation files | `.fs`, `.fsx` |
| `LANGUAGE_SIGNATURE` | signature files      | `.fsi`        |

## Installation

```bash
cargo add tree-sitter tree-sitter-fsharp
```

The parsers are compiled from the bundled C sources by `build.rs`, so a C compiler is
required. The crate itself depends only on `tree-sitter-language` 0.1 and does not pull in
a runtime, so you choose the `tree-sitter` version; the crate is tested against 0.26.

## Usage

```rust
let code = r#"
module M =
  let x = 0
"#;

let mut parser = tree_sitter::Parser::new();
parser
    .set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
    .expect("Error loading F# parser");

let tree = parser.parse(code, None).unwrap();
assert!(!tree.root_node().has_error());
```

Signature files need the other grammar — it is a separate parser, not a mode of the first:

```rust
parser
    .set_language(&tree_sitter_fsharp::LANGUAGE_SIGNATURE.into())
    .expect("Error loading F# signature parser");

let tree = parser.parse("module M\nval x : int\n", None).unwrap();
```

## Queries and node types

Query sources and the static node type descriptions are exposed as `&'static str`
constants:

- `HIGHLIGHTS_QUERY`, `INJECTIONS_QUERY`, `LOCALS_QUERY`, `TAGS_QUERY`
- `FSHARP_NODE_TYPES`, `SIGNATURE_NODE_TYPES` — the contents of the respective
  [`node-types.json`][node-types] files

## Contributing and grammar development

Grammar sources, the test corpus and the development workflow live in the repository:
see the [main README][repo] and [AGENTS.md][agents]. Bug reports for mis-parsed F# code
are welcome — reduce the snippet to a minimal example and open an issue or PR.

## License

MIT — see [LICENSE][license].

[tree-sitter]: https://tree-sitter.github.io/tree-sitter/
[node-types]: https://tree-sitter.github.io/tree-sitter/using-parsers/6-static-node-types.html
[repo]: https://github.com/ionide/tree-sitter-fsharp#readme
[agents]: https://github.com/ionide/tree-sitter-fsharp/blob/main/AGENTS.md
[license]: https://github.com/ionide/tree-sitter-fsharp/blob/main/LICENSE
