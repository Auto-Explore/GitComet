/**
 * @file CSV grammar for tree-sitter
 * @author Amaan Qureshi <amaanq12@gmail.com>
 * @license MIT
 */

// GitComet: `./common/`, not `../common/`. Upstream is a three-grammar
// repository whose `csv/`, `psv/` and `tsv/` grammars share a module one level
// up; only `csv/` is vendored, so that module sits beside this file instead.
const defineGrammar = require('./common/define-grammar');

module.exports = defineGrammar('csv', ',');
