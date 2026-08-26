/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

import ocaml from '../ocaml/grammar.js';

const ocaml_type_grammar = grammar(ocaml, {
  name: 'ocaml_type',

  rules: {
    type: $ => $._type,
  },
});

// Make 'type' the first rule
ocaml_type_grammar.grammar.rules = Object.assign(
  {type: null},
  ocaml_type_grammar.grammar.rules,
);

export default ocaml_type_grammar;
