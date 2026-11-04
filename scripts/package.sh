#!/bin/sh
# Build installable packages: scripts/package.sh print3d track   (no arguments: every plugin)
# Each becomes dist/plannr-plugin-<id>-<version>.plannr-plugin, a zip holding plugin.json, engine.wasm
# (the Rust engine for wasm32-wasip1), web.js (the browser half), README.md and LICENSE.
# Needs: git submodule update --init, rustup target add wasm32-wasip1, npm install, zip.
set -eu
cd "$(dirname "$0")/.."
[ -f plannr/crates/plannr-core/Cargo.toml ] || { echo "the plannr submodule is missing: git submodule update --init" >&2; exit 1; }
if [ $# -eq 0 ]; then set -- $(for f in */plugin.json; do dirname "$f"; done); fi
out="$PWD/dist"; mkdir -p "$out"
for id in "$@"; do
  [ -f "$id/plugin.json" ] || { echo "$id/plugin.json missing" >&2; exit 1; }
  version=$(node -p "require('./$id/plugin.json').version")
  echo "== $id $version"
  tmp=$(mktemp -d)
  cp "$id/plugin.json" "$tmp/plugin.json"
  if [ -d "$id/engine" ]; then
    crate=$(sed -n 's/^name = "\(.*\)"/\1/p' "$id/engine/Cargo.toml" | head -1); lib=$(echo "$crate" | tr - _)
    cargo build --release --target wasm32-wasip1 -p "$crate"
    cp "target/wasm32-wasip1/release/$lib.wasm" "$tmp/engine.wasm"
  fi
  if [ -f "$id/web/index.ts" ]; then
    PLUGIN="$id" npx vite build --logLevel warn
    cp "$id/dist/web.js" "$tmp/web.js"
  fi
  [ -f "$id/README.md" ] && cp "$id/README.md" "$tmp/README.md"
  cp LICENSE "$tmp/LICENSE"
  pkg="$out/plannr-plugin-$id-$version.plannr-plugin"; rm -f "$pkg"
  (cd "$tmp" && zip -q -X -r "$pkg" .)
  rm -r "$tmp"
  echo "   $pkg ($(du -h "$pkg" | cut -f1 | tr -d ' '))"
done
