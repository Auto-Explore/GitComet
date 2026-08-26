# tree-sitter-ocaml

[![CI][ci]](https://github.com/tree-sitter/tree-sitter-ocaml/actions/workflows/ci.yml)
[![crates][crates]](https://crates.io/crates/tree-sitter-ocaml)

This crate provides OCaml language support for the [tree-sitter][] parsing
library. There are separate languages for implementations (`.ml`),
interfaces (`.mli`) and types.

## Installation

To use this crate, add it to the `[dependencies]` section of your `Cargo.toml`
file. (Note that you will probably also need to depend on the
[`tree-sitter`][tree-sitter crate] crate to use the parsed result in any useful
way.)

```toml
[dependencies]
tree-sitter = "0.25"
tree-sitter-ocaml = "0.25"
```

## Usage

Typically, you will use [`LANGUAGE_OCAML`][LANGUAGE_OCAML] to add this language
to a tree-sitter [`Parser`][Parser], and then use the parser to parse some code:

```rust
let code = r#"
    module M = struct
      let x = 0
    end
"#;
let mut parser = tree_sitter::Parser::new();
let language = tree_sitter_ocaml::LANGUAGE_OCAML;
parser
    .set_language(&language.into())
    .expect("Error loading OCaml language");
let tree = parser.parse(code, None).unwrap();
```

Use [`LANGUAGE_OCAML_INTERFACE`][LANGUAGE_OCAML_INTERFACE] to parse interface
files (with `.mli` extension) and [`LANGUAGE_OCAML_TYPE`][LANGUAGE_OCAML_TYPE]
to parse type signatures.

[ci]: https://img.shields.io/github/actions/workflow/status/tree-sitter/tree-sitter-ocaml/ci.yml?logo=github&label=CI
[crates]: https://img.shields.io/crates/v/tree-sitter-ocaml?logo=rust
[tree-sitter]: https://tree-sitter.github.io/
[tree-sitter crate]: https://crates.io/crates/tree-sitter
[LANGUAGE_OCAML]: https://docs.rs/tree-sitter-ocaml/*/tree_sitter_ocaml/constant.LANGUAGE_OCAML.html
[LANGUAGE_OCAML_INTERFACE]: https://docs.rs/tree-sitter-ocaml/*/tree_sitter_ocaml/constant.LANGUAGE_OCAML_INTERFACE.html
[LANGUAGE_OCAML_TYPE]: https://docs.rs/tree-sitter-ocaml/*/tree_sitter_ocaml/constant.LANGUAGE_OCAML_TYPE.html
[Parser]: https://docs.rs/tree-sitter/*/tree_sitter/struct.Parser.html
