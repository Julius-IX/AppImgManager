#!/usr/bin/env bash
# Build release binaries for all targets and drop them in builds/
# using the exact asset names install.sh expects.
#
#   ./scripts/build.sh
#
# Cross-compiling (e.g. aarch64 from an x86_64 box) needs zig + cargo-zigbuild:
#   cargo install cargo-zigbuild  (and install zig from package manager)

set -euo pipefail

cd "$(dirname "$0")/.." # project root (this script SHOULD live in scripts/)

name="app-img"
out="builds"
# Binary name from Cargo.toml; override with BIN=whatever ./scripts/build.sh
bin="${BIN:-$(sed -n 's/^name *= *"\(.*\)"/\1/p' Cargo.toml | head -1)}"

# rust target -> release asset name (must match install.sh: $name-$arch-linux)
targets=(
  "x86_64-unknown-linux-musl:$name-x86_64-linux"
  "aarch64-unknown-linux-musl:$name-aarch64-linux"
)

host_arch=$(uname -m)
if command -v cargo-zigbuild >/dev/null; then
  build=(cargo zigbuild --release)
  can_cross=1
else
  build=(cargo build --release)
  can_cross=0
fi

rm -rf "$out"
mkdir -p "$out"

for entry in "${targets[@]}"; do
  target="${entry%%:*}"
  asset="${entry#*:}"
  arch="${target%%-*}"

  if [ "$can_cross" -eq 0 ] && [ "$arch" != "$host_arch" ]; then
    echo "SKIP  $target (cross-compiling needs cargo-zigbuild + zig)"
    continue
  fi

  echo "BUILD $target"
  rustup target add "$target"
  "${build[@]}" --target "$target"
  install -Dm755 "target/$target/release/$bin" "$out/$asset"
done

echo
echo "Done:"
ls -lh "$out"
