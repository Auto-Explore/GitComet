/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

import ocaml from '../ocaml/grammar.js';

export default grammar(ocaml, {
  name: 'ocaml_interface',

  rules: {
    compilation_unit: $ => optional($._signature),
  },
});
