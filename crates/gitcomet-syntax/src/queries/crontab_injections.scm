; Everything after the fifth field is a shell command, and the grammar hands it
; over as one opaque token -- so without this the half of each line that people
; actually read renders as plain text. Same reasoning as
; queries/makefile_injections.scm.
;
; Not `injection.combined`: each cron entry is its own invocation, so one document
; per command is the semantics, and it keeps an unbalanced quote in one job from
; recolouring the next.
;
; One thing bash will get wrong, and cron users already know it: a `%` in a cron
; command is a newline, not a literal percent. That is the classic crontab bug --
; `date +%Y` truncates the command -- and no shell grammar can show it.
((command) @injection.content
  (#set! injection.language "bash"))
