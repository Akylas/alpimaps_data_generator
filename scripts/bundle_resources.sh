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

# --- valhalla_build_tiles --------------------------------------------------------------------
# The only external tool left: the graph builder. `valhalla_build_elevation` is not here on
# purpose - it is a Python script, and downloading .hgt tiles is done natively instead, so the
# app needs no interpreter.
#
# The binary links ~95 Homebrew dylibs, so copying it alone would produce something that runs
# only on a machine with the same Homebrew installation. It gets the same treatment as the app:
# its dependencies are collected into Frameworks and its load commands rewritten. From
# Resources/valhalla/ that is one level up, hence the different prefix.
mkdir -p "$resources/valhalla"
if [ -x "$repo/valhalla/build/valhalla_build_tiles" ]; then
  cp "$repo/valhalla/build/valhalla_build_tiles" "$resources/valhalla/valhalla_build_tiles"
  if [ "$(uname)" = Darwin ]; then
    "$repo/scripts/bundle_macos_dylibs.sh" \
      "$resources/valhalla/valhalla_build_tiles" \
      "$resources/Frameworks" \
      "@executable_path/../Frameworks"
  fi
  say "valhalla_build_tiles"
else
  echo "  note: valhalla_build_tiles not built; the packaged app will look for it on PATH and" >&2
  echo "        say so in Docs -> Where things live if it is not there either" >&2
fi

# --- dylibs (macOS) --------------------------------------------------------------------------
# the app links Valhalla, whose dependencies are Homebrew dylibs that will not exist on a user's
# machine; this rewrites them to load from inside the bundle
if [ "$(uname)" = Darwin ]; then
  "$repo/scripts/bundle_macos_dylibs.sh" \
    "$studio/target/release/alpimaps-studio" \
    "$resources/Frameworks" \
    "@executable_path/../Resources/Frameworks"
fi
