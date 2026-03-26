#!/usr/bin/env bash
set -euo pipefail

prefix="${HOME}/.local"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix) prefix="$2"; shift 2 ;;
    -h|--help)
      echo "Usage: scripts/uninstall-linux.sh [--prefix PATH]"
      exit 0
      ;;
    *) echo "Unknown arg: $1" >&2; exit 2 ;;
  esac
done

bindir="${prefix}/bin"
appdir="${XDG_DATA_HOME:-${HOME}/.local/share}/applications"
iconsroot="${XDG_DATA_HOME:-${HOME}/.local/share}/icons/hicolor"
icon_sizes=(32 48 128 256 512)
desktop_files=("dev.gitcomet.GitComet.desktop" "gitcomet.desktop")
icon_names=("dev.gitcomet.GitComet" "gitcomet")

rm -f "${bindir}/gitcomet"
rm -f "${bindir}/gitcomet-app"
for desktop_file in "${desktop_files[@]}"; do
  rm -f "${appdir}/${desktop_file}"
done
for size in "${icon_sizes[@]}"; do
  for icon_name in "${icon_names[@]}"; do
    rm -f "${iconsroot}/${size}x${size}/apps/${icon_name}.png"
  done
done

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$appdir" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache "${iconsroot}" >/dev/null 2>&1 || true

echo "Uninstalled GitComet desktop integration from ${prefix} and ~/.local/share."
