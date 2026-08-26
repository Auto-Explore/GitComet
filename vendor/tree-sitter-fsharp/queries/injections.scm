;; Literate documentation comments: (** ... *)
;; The FSharp.Formatting convention treats these as Markdown. The content node
;; begins with the extra `*` of the `(**` opener, so we match on that to tell a
;; doc comment apart from an ordinary `(* ... *)` block comment, and offset by
;; one column to drop the leading `*` before handing it to Markdown.
;; Markdown's own injections then highlight fenced code blocks (```sql, ...).
((block_comment_content) @injection.content
 (#match? @injection.content "^\\*")
 (#offset! @injection.content 0 1 0 0)
 (#set! injection.language "markdown"))

;; Ordinary comments
((block_comment_content) @injection.content
 (#not-match? @injection.content "^\\*")
 (#set! injection.language "comment"))

((line_comment) @injection.content
 (#set! injection.language "comment"))

((xml_doc) @injection.content
 (#offset! @injection.content 0 3 0 0)
 (#set! injection.language "xml")
 (#set! injection.combined))
