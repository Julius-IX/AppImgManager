#!/usr/bin/env bash
# Uninstall (installed copy, works offline):
#   sudo /usr/local/share/app-img/uninstall.sh
#
# Or straight from the repo:
#   curl -fsSL https://raw.githubusercontent.com/Julius-IX/AppImgManager/main/scripts/uninstall.sh | bash

set -euo pipefail

main() {
  local name="app-img"
  local icon_file="appimage-icon.svg"

  local SUDO=""
  if [ "$(id -u)" -ne 0 ]; then
    command -v sudo >/dev/null || {
      echo "Need root or sudo." >&2
      exit 1
    }
    SUDO="sudo"
  fi

  $SUDO rm -f "/usr/local/bin/$name" \
    "/usr/share/applications/$name.desktop" \
    "/usr/share/icons/hicolor/scalable/apps/$icon_file" \
    "/usr/share/pixmaps/$name.png" # leftover from older installs
  $SUDO rm -rf "/usr/local/share/$name"

  if command -v gtk-update-icon-cache >/dev/null; then
    $SUDO gtk-update-icon-cache -f -t /usr/share/icons/hicolor || true
  fi
  if command -v update-desktop-database >/dev/null; then
    $SUDO update-desktop-database /usr/share/applications || true
  fi

  echo "Removed $name."
}

main "$@"
