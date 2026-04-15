#!/usr/bin/env sh
set -eu

host_home=""
if [ -n "${FLATPAK_ID:-}" ] && [ -n "${HOME:-}" ]; then
  suffix="/.var/app/${FLATPAK_ID}"
  case "$HOME" in
    *"$suffix")
      host_home="${HOME%$suffix}"
      ;;
  esac
fi

set -- git "$@"

if [ -n "${host_home:-}" ]; then
  set -- "HOME=${host_home}" "$@"
fi

[ -n "${HOST_XDG_CONFIG_HOME:-}" ] && set -- "XDG_CONFIG_HOME=${HOST_XDG_CONFIG_HOME}" "$@"
[ -n "${HOST_XDG_DATA_HOME:-}" ] && set -- "XDG_DATA_HOME=${HOST_XDG_DATA_HOME}" "$@"
[ -n "${HOST_XDG_CACHE_HOME:-}" ] && set -- "XDG_CACHE_HOME=${HOST_XDG_CACHE_HOME}" "$@"
[ -n "${HOST_XDG_STATE_HOME:-}" ] && set -- "XDG_STATE_HOME=${HOST_XDG_STATE_HOME}" "$@"
[ -n "${DISPLAY:-}" ] && set -- "DISPLAY=${DISPLAY}" "$@"
[ -n "${WAYLAND_DISPLAY:-}" ] && set -- "WAYLAND_DISPLAY=${WAYLAND_DISPLAY}" "$@"
[ -n "${GIT_ASKPASS:-}" ] && set -- "GIT_ASKPASS=${GIT_ASKPASS}" "$@"
[ -n "${SSH_ASKPASS:-}" ] && set -- "SSH_ASKPASS=${SSH_ASKPASS}" "$@"
[ -n "${SSH_ASKPASS_REQUIRE:-}" ] && set -- "SSH_ASKPASS_REQUIRE=${SSH_ASKPASS_REQUIRE}" "$@"
[ -n "${GIT_TERMINAL_PROMPT:-}" ] && set -- "GIT_TERMINAL_PROMPT=${GIT_TERMINAL_PROMPT}" "$@"
[ -n "${GITCOMET_ASKPASS_PROMPT_LOG:-}" ] && set -- "GITCOMET_ASKPASS_PROMPT_LOG=${GITCOMET_ASKPASS_PROMPT_LOG}" "$@"
[ -n "${GITCOMET_AUTH_KIND:-}" ] && set -- "GITCOMET_AUTH_KIND=${GITCOMET_AUTH_KIND}" "$@"
[ -n "${GITCOMET_AUTH_USERNAME:-}" ] && set -- "GITCOMET_AUTH_USERNAME=${GITCOMET_AUTH_USERNAME}" "$@"
[ -n "${GITCOMET_AUTH_SECRET:-}" ] && set -- "GITCOMET_AUTH_SECRET=${GITCOMET_AUTH_SECRET}" "$@"

exec flatpak-spawn --host env "$@"
