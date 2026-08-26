; The whole doc comment reads as a comment first; the captures below then take
; the spans they name back off it.
;
; Without this base capture a prose-only `/** ... */` renders with no colour at
; all: the host grammar's `(comment) @comment` is subtracted from the document
; before an injection paints, so an injection that captures nothing leaves the
; bytes bare. Ordering matters -- overlapping captures resolve last-wins, so
; this has to come first.
(document) @comment

(tag_name) @keyword.jsdoc

(type) @type.jsdoc

(identifier) @variable.jsdoc
