; JinjaText is the non-markup reading of a template (`values.yaml.j2`,
; `deploy.sh.j2`): it drops only the HTML rule of jinja_injections.scm. Front
; matter is body-agnostic, so it keeps that one.
((front_matter) @injection.content
 (#set! injection.language "yaml"))
