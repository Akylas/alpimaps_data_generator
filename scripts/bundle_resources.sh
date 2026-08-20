#!/usr/bin/env bash
#
# Collect everything the packaged app needs to run without this repository.
#
# A published build has no submodules, no scripts and no checkout: whatever the pipeline reaches
# for at run time has to be inside the bundle. Run from `alpimaps-studio/` by Tauri's
# `beforeBundleCommand`.
#
#   planetiler jar   the basemap and routes steps are a subprocess over this
#   valhalla.json    the embedded router validates the whole document, so a stub will not do
#   alpimaps         the CLI, and the in-app reference reads its --help
#   valhalla/*       the two build tools, when they have been built
#
# What is deliberately *not* bundled: the OSM extracts, the elevation tiles and the Valhalla
# graph. Those are gigabytes, they are downloaded or built per area, and they belong in the
# user's data directories rather than inside an app.

set -euo pipefail

studio="$(cd "$(dirname "${BASH_SOURCE[0]}")/../alpimaps-studio" && pwd)"
repo="$(dirname "$studio")"
resources="$studio/src-tauri/resources"
mkdir -p "$resources"

say() { printf '  %s\n' "$*"; }

# --- the CLI ---------------------------------------------------------------------------------
( cd "$studio" && cargo build --release -p alpimaps-cli )
cp "$studio/target/release/alpimaps" "$resources/alpimaps"
say "alpimaps"

# --- planetiler ------------------------------------------------------------------------------
# the newest built jar; PLANETILER_JAR overrides, which is how CI points at a downloaded one
jar="${PLANETILER_JAR:-}"
if [ -z "$jar" ]; then
  jar="$(ls -1 "$repo"/planetiler/planetiler-dist/target/*-with-deps.jar 2>/dev/null | sort | tail -1 || true)"
fi
if [ -n "$jar" ] && [ -f "$jar" ]; then
  cp "$jar" "$resources/$(basename "$jar")"
  say "$(basename "$jar")"
else
  echo "  WARNING: no planetiler jar found; the packaged app will not build basemaps" >&2
fi

# --- valhalla.json ---------------------------------------------------------------------------
if [ -f "$repo/valhalla.json" ]; then
  cp "$repo/valhalla.json" "$resources/valhalla.json"
  say "valhalla.json"
else
  echo "  WARNING: no valhalla.json; routing steps will have no config to start from" >&2
fi

# --- valhalla tools --------------------------------------------------------------------------
# optional: without them the app still runs, and the two steps that need them say what is
# missing rather than failing halfway
mkdir -p "$resources/valhalla"
for tool in valhalla_build_tiles valhalla_build_elevation; do
  if [ -x "$repo/valhalla/build/$tool" ]; then
    cp "$repo/valhalla/build/$tool" "$resources/valhalla/$tool"
    say "$tool"
  else
    echo "  note: $tool not built; the packaged app will look for it on PATH" >&2
  fi
done

# --- dylibs (macOS) --------------------------------------------------------------------------
# the app links Valhalla, whose dependencies are Homebrew dylibs that will not exist on a user's
# machine; this rewrites them to load from inside the bundle
if [ "$(uname)" = Darwin ]; then
  "$repo/scripts/bundle_macos_dylibs.sh" \
    "$studio/target/release/alpimaps-studio" \
    "$resources/Frameworks" \
    "@executable_path/../Resources/Frameworks"
fi
