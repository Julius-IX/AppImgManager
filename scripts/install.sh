#!/usr/bin/env bash
# Install:
#   curl -fsSL https://raw.githubusercontent.com/Julius-IX/AppImgManager/main/scripts/install.sh | bash
#
# Pin a specific version:
#   curl -fsSL https://raw.githubusercontent.com/Julius-IX/AppImgManager/main/scripts/install.sh | VERSION=v1.0.0 bash

set -euo pipefail

main() {
  local name="app-img"
  local repo="Julius-IX/AppImgManager"
  local scripts_dir="scripts"
  local icon_file="appimage-icon.svg" # lives at the repo root
  local icon_path="/usr/share/icons/hicolor/scalable/apps/$icon_file"
  local version="${VERSION:-latest}"
  local arch
  arch=$(uname -m) # x86_64 or aarch64

  local release_url
  if [ "$version" = "latest" ]; then
    release_url="https://github.com/$repo/releases/latest/download"
    # Resolve the latest tag so the repo files match the binary version
    version=$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
      "https://github.com/$repo/releases/latest" | sed 's|.*/||')
  else
    release_url="https://github.com/$repo/releases/download/$version"
  fi
  local raw_base="https://raw.githubusercontent.com/$repo/$version"

  local SUDO=""
  if [ "$(id -u)" -ne 0 ]; then
    command -v sudo >/dev/null || {
      echo "Need root or sudo." >&2
      exit 1
    }
    SUDO="sudo"
  fi

  local tmp
  tmp=$(mktemp -d)
  # Double quotes: expand $tmp NOW. With single quotes, the trap runs after
  # main() returns, $tmp is out of scope, and set -u kills it.
  trap "rm -rf '$tmp'" EXIT

  echo "Installing $name $version ($arch)"
  curl -fsSL "$release_url/$name-$arch-linux" -o "$tmp/$name"
  curl -fsSL "$raw_base/$icon_file" -o "$tmp/$icon_file"
  curl -fsSL "$raw_base/$scripts_dir/uninstall.sh" -o "$tmp/uninstall.sh"

  $SUDO install -Dm755 "$tmp/$name" "/usr/local/bin/$name"
  $SUDO install -Dm644 "$tmp/$icon_file" "$icon_path"
  $SUDO install -Dm755 "$tmp/uninstall.sh" "/usr/local/share/$name/uninstall.sh"

  $SUDO tee "/usr/share/applications/$name.desktop" >/dev/null <<EOF
[Desktop Entry]
Type=Application
Name=AppImage Manager
Comment=Manage AppImages
Exec=/usr/local/bin/$name gui %f
Icon=$icon_path
Terminal=false
MimeType=application/vnd.appimage;application/x-iso9660-appimage;
Categories=Utility;
EOF

  if command -v gtk-update-icon-cache >/dev/null; then
    $SUDO gtk-update-icon-cache -f -t /usr/share/icons/hicolor || true
  fi
  if command -v update-desktop-database >/dev/null; then
    $SUDO update-desktop-database /usr/share/applications || true
  fi

  echo "Done. $name now appears under \"Open with\" for AppImages."
  echo "Uninstall with: sudo /usr/local/share/$name/uninstall.sh"
  echo "  or: curl -fsSL https://raw.githubusercontent.com/$repo/main/$scripts_dir/uninstall.sh | bash"
}

main "$@"
