; crontab.
;
; Authored here alongside vendor/tree-sitter-crontab, which was written for
; GitComet because no crontab grammar exists anywhere with a usable licence.
;
; The command is not captured: queries/crontab_injections.scm hands it to bash,
; which is what it is.

(comment) @comment

; `MAILTO=""`, `PATH=/usr/bin` -- read by cron itself, never by a shell.
(environment
  name: (variable) @property
  value: (value)? @string)

; `@reboot`, `@daily` -- these replace all five fields at once.
(special) @keyword

; The five fields. `*` is the one that carries meaning by itself.
(wildcard) @punctuation.special
(number) @number

; `JAN`, `MON` -- the named months and weekdays.
(name) @constant.builtin

(range "-" @operator)
(step "/" @operator)

"," @punctuation.delimiter
"=" @operator
