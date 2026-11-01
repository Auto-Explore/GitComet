(comment) @comment

(document
  (string) @string)

(document
  (string) @property.json_key
  .
  (ERROR) @_json_key_colon
  (#eq? @_json_key_colon ":"))

(pair
  value: (string) @string)

(array
  (string) @string)

(escape_sequence) @string.escape

(pair
  key: (string) @property.json_key)

(number) @number

[
  (true)
  (false)
] @boolean

(null) @constant.builtin

[
  ","
  ":"
] @punctuation.delimiter

[
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket
