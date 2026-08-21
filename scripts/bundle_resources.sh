#!/usr/bin/env bash
#
# Collect everything the packaged app needs to run without this repository.
#
# A published build has no submodules, no scripts and no checkout: whatever the pipeline reaches
# for at run time has to be inside the bundle. Run from `cairn/` by Tauri's
# `beforeBundleCommand`.
#
#   planetiler jar   the basemap and routes steps are a subprocess over this
#   valhalla.json    the embedded router validates the whole document, so a stub will not do
#   cairn            the CLI, and the in-app reference reads its --help
#   valhalla/*       the two build tools, when they have been built
#
# What is deliberately *not* bundled: the OSM extracts, the elevation tiles and the Valhalla
# graph. Those are gigabytes, they are downloaded or built per area, and they belong in the
# user's data directories rather than inside an app.

set -euo pipefail

studio="$(cd "$(dirname "${BASH_SOURCE[0]}")/../cairn" && pwd)"
repo="$(dirname "$studio")"
resources="$studio/src-tauri/resources"
# the dylibs live in their own source directory: mapping one directory to two destinations put
# 34 MB of them in the bundle twice
#
# Linux uses the same directory for the same purpose - the libraries the app carries - so the
# name is macOS's rather than a second resource mapping in tauri.conf.json for one .so directory.
frameworks="$studio/src-tauri/frameworks"
mkdir -p "$resources" "$frameworks"

# Where those two directories end up once installed decides every rpath below.
#
#   macOS   Cairn.app/Contents/Resources/{resources,Frameworks}
#   Linux   /usr/bin/cairn and /usr/lib/<product name>/{resources,Frameworks}
#
# The Linux layout is debian::generate_data in tauri-bundler, and the AppImage bundler copies
# that same usr/ tree into the AppDir - so one set of rpaths serves .deb, .rpm and .AppImage.
product="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["productName"])' \
  "$studio/src-tauri/tauri.conf.json")"

say() { printf '  %s\n' "$*"; }

# --- the CLI ---------------------------------------------------------------------------------
( cd "$studio" && cargo build --release -p cairn-cli )
# built as cairn-cli so that this build does not overwrite the app binary, which Tauri is
# about to bundle from target/release/cairn - see cli/Cargo.toml
cp "$studio/target/release/cairn-cli" "$resources/cairn"
# it links Valhalla too, so it needs the same treatment as the app - on the build machine it
# runs either way, which is exactly why this was missed until a bundle was checked
if [ "$(uname)" = Darwin ]; then
  # Contents/Resources/resources -> Contents/Resources/Frameworks
  "$repo/scripts/bundle_macos_dylibs.sh" \
    "$resources/cairn" \
    "$frameworks" \
    "@executable_path/../Frameworks" >/dev/null
else
  # /usr/lib/<product>/resources -> /usr/lib/<product>/Frameworks
  "$repo/scripts/bundle_linux_libs.sh" \
    "$resources/cairn" \
    "$frameworks" \
    '$ORIGIN/../Frameworks' >/dev/null
fi
say "cairn"

# --- planetiler ------------------------------------------------------------------------------
# BUNDLE_JAR=0 leaves it out entirely: the app then fetches one on first use into its data
# directory, from the URL in Settings. That is 89 MB off the download for an install that only
# inspects tiles - but it needs a jar published somewhere to fetch, and this pipeline runs a
# *fork* of planetiler, so the URL cannot default to planetiler's own releases: a jar from there
# builds a different schema without saying so.
if [ "${BUNDLE_JAR:-1}" = "0" ]; then
  rm -f "$resources"/*-with-deps.jar
  say "planetiler jar: not bundled (BUNDLE_JAR=0)"
else
  jar="${PLANETILER_JAR:-}"
  if [ -z "$jar" ]; then
    jar="$(ls -1 "$repo"/planetiler/planetiler-dist/target/*-with-deps.jar 2>/dev/null | sort | tail -1 || true)"
  fi
  if [ -n "$jar" ] && [ -f "$jar" ]; then
    # the fat jar carries native libraries for every platform at once; keep this one's
    if [ "${STRIP_JAR:-1}" = "1" ] && command -v python3 >/dev/null; then
      python3 "$repo/scripts/strip_jar_natives.py" "$jar" "$resources/$(basename "$jar")"
    else
      cp "$jar" "$resources/$(basename "$jar")"
    fi
    say "$(basename "$jar")"
  else
    echo "  WARNING: no planetiler jar found; the packaged app cannot build basemaps until one" >&2
    echo "           is fetched from the URL in Settings" >&2
  fi
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
    # Tauri's resource map is rooted at Contents/Resources, so `frameworks/ -> Frameworks/`
    # lands at Contents/Resources/Frameworks. From Contents/Resources/resources/valhalla that is
    # two levels up.
    "$repo/scripts/bundle_macos_dylibs.sh" \
      "$resources/valhalla/valhalla_build_tiles" \
      "$frameworks" \
      "@executable_path/../../Frameworks"
  else
    # /usr/lib/<product>/resources/valhalla -> /usr/lib/<product>/Frameworks
    "$repo/scripts/bundle_linux_libs.sh" \
      "$resources/valhalla/valhalla_build_tiles" \
      "$frameworks" \
      '$ORIGIN/../../Frameworks' >/dev/null
  fi
  say "valhalla_build_tiles"
else
  echo "  note: valhalla_build_tiles not built; the packaged app will look for it on PATH and" >&2
  echo "        say so in Docs -> Where things live if it is not there either" >&2
fi

# --- the libraries the app itself needs ------------------------------------------------------
# The app links Valhalla, and Valhalla's dependencies are not something a user's machine can be
# relied on to have: Homebrew dylibs on macOS, and on Linux libraries whose sonames change
# between releases (libprotobuf.so.23 on Ubuntu 22.04, .32 on 24.04). Either way they travel
# with the bundle and the binary is pointed at them.
if [ "$(uname)" = Darwin ]; then
  # Contents/MacOS -> Contents/Resources/Frameworks
  "$repo/scripts/bundle_macos_dylibs.sh" \
    "$studio/target/release/cairn" \
    "$frameworks" \
    "@executable_path/../Resources/Frameworks"
else
  # /usr/bin -> /usr/lib/<product>/Frameworks
  "$repo/scripts/bundle_linux_libs.sh" \
    "$studio/target/release/cairn" \
    "$frameworks" \
    "\$ORIGIN/../lib/$product/Frameworks" >/dev/null
  # the total, not what this call added: the CLI and the graph builder were bundled first and
  # share the directory, so most of it is already there by now
  say "$(ls -1 "$frameworks" | wc -l | tr -d ' ') libraries carried into Frameworks"
fi
