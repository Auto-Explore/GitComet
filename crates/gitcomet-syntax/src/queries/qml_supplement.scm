; QML -- a supplement, appended to queries/typescript_highlights.scm.
;
; tree-sitter-qmljs extends tree-sitter-typescript's grammar, and its own
; tree-sitter.json builds the QML query as javascript + typescript + this file.
; GitComet's typescript_highlights.scm already covers both JS and TS and compiles
; against this grammar, so this is wired as
; `with_supplement(TYPESCRIPT_HIGHLIGHTS_QUERY, <this>)`.
;
; The first part is tree-sitter-qmljs 0.3.1's queries/highlights.scm (MIT,
; https://github.com/yuja/tree-sitter-qmljs), verbatim. Everything after
; "GitComet additions" is authored here from the grammar's node types: upstream
; colours no object type (`Rectangle {`), so the most common token in a QML file
; rendered as a plain variable, and import paths read the same.

;;; Annotations

(ui_annotation
  "@" @operator
  type_name: [
    (identifier) @attribute
    (nested_identifier (identifier) @attribute)
  ])

;;; Properties

(ui_object_definition_binding
  name: [
    (identifier) @property
    (nested_identifier (identifier) @property)
  ])

(ui_binding
  name: [
    (identifier) @property
    (nested_identifier (identifier) @property)
  ])

;; locals query appears not working unless id: <ref> isn't a parameter.
(ui_binding
  name: (identifier) @property
  (#eq? @property "id")
  value: (expression_statement (identifier) @variable.parameter))

(ui_property
  name: (identifier) @property)

(ui_required
  name: (identifier) @property)

(ui_list_property_type
  ["<" ">"] @punctuation.bracket)

;;; Signals

(ui_signal
  name: (identifier) @function.signal)

(ui_signal_parameter
  (identifier) @variable.parameter)

;;; Keywords

[
  "as"
  "component"
  "default"
  "final"
  "import"
  "on"
  "override"
  "pragma"
  "property"
  "readonly"
  "required"
  "signal"
  "virtual"
] @keyword

;;; GitComet additions

; Object types: `Rectangle {`, `L.ColumnLayout {`, `Behavior on x {`, and an
; inline `component Name: ...`. A qualified type's prefix is its import alias.
(ui_object_definition
  type_name: (identifier) @type)

(ui_object_definition_binding
  type_name: (identifier) @type)

(ui_object_definition
  type_name: (nested_identifier
    .
    (identifier) @namespace))

(ui_object_definition_binding
  type_name: (nested_identifier
    .
    (identifier) @namespace))

(ui_object_definition
  type_name: (nested_identifier
    (identifier) @type
    .))

(ui_object_definition_binding
  type_name: (nested_identifier
    (identifier) @type
    .))

(ui_inline_component
  name: (identifier) @type)

; `import QtQuick.Controls 2.15 as C`: the module path and its alias. The path
; nests one `nested_identifier` per dot, and the typescript part's
; `(identifier) @variable` wins over a capture of the whole path, so each depth
; is spelled out -- enough for `QtQuick.Controls.Material.impl`.
(ui_import
  source: (identifier) @namespace)

(ui_import
  source: (nested_identifier
    (identifier) @namespace))

(ui_import
  source: (nested_identifier
    (nested_identifier
      (identifier) @namespace)))

(ui_import
  source: (nested_identifier
    (nested_identifier
      (nested_identifier
        (identifier) @namespace))))

(ui_import
  alias: (identifier) @namespace)
