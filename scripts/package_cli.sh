#!/usr/bin/env bash
#
# Package the `cairn` command line on its own, for people who want the pipeline without the app.
#
#   scripts/package_cli.sh <version> [outdir]
#
# The binary links Valhalla, which on macOS means a hundred Homebrew dylibs that exist on the
# build machine and nowhere else - so the archive carries them in `lib/` and the binary is
# rewritten to load from there. Run this *before* the app bundle is built: `bundle_resources.sh`
# rewrites the same binary for a different layout, and a binary can only point one way at a time.
set -euo pipefail

VERSION="${1:?usage: package_cli.sh <version> [outdir]}"
OUT="${2:-dist}"

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
studio="$repo/cairn"
binary="$studio/target/release/cairn"

[ -f "$binary" ] || { echo "no CLI at $binary - build it first" >&2; exit 1; }

case "$(uname)-$(uname -m)" in
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64) target="aarch64-unknown-linux-gnu" ;;
  *) target="$(uname -s)-$(uname -m)" ;;
esac

name="cairn-${VERSION}-${target}"
stage="$OUT/$name"
rm -rf "$stage"
mkdir -p "$stage"
cp "$binary" "$stage/cairn"

if [ "$(uname)" = Darwin ]; then
  # a copy is rewritten, never the build output: the app bundle needs that one pristine
  "$repo/scripts/bundle_macos_dylibs.sh" "$stage/cairn" "$stage/lib" "@executable_path/lib" \
    >/dev/null
  echo "  carried $(ls -1 "$stage/lib" | wc -l | tr -d ' ') libraries into lib/"
fi

cat > "$stage/README.txt" <<EOF
cairn $VERSION ($target)

The AlpiMaps tile pipeline from a terminal: OSM extracts, basemap and route tiles, terrain-RGB,
Valhalla routing packages.

  ./cairn --help
  ./cairn catalog --stats
  ./cairn basemap --area rhone-alpes --repo /path/to/alpimaps_data_generator

Put it anywhere on PATH. Java 21+ has to be available for the planetiler steps; the jar is
fetched on first use unless one is passed with --jar.
$(if [ "$(uname)" = Darwin ]; then
cat <<'MAC'

lib/ holds the libraries Valhalla needs. Keep it next to the binary - the binary looks for it
there, so moving `cairn` on its own leaves routing unable to start.
MAC
else
cat <<'LINUX'

Dynamically linked against the usual system libraries (libcurl, libsqlite3, libspatialite, geos,
luajit, protobuf). Built on Ubuntu 24.04, so it needs glibc 2.39 or newer - 22.04 is too old.
On Debian/Ubuntu:

  apt install libsqlite3-0 libspatialite8 libgeos-c1v5 libluajit-5.1-2 libprotobuf-lite32 libcurl4
LINUX
fi)
EOF

archive="$OUT/${name}.tar.gz"
tar -czf "$archive" -C "$OUT" "$name"
( cd "$OUT" && shasum -a 256 "${name}.tar.gz" > "${name}.tar.gz.sha256" )

echo "  $archive ($(du -h "$archive" | cut -f1))"

# --- prove the archive, not the staging directory ---------------------------------------------
#
# They differ in exactly the way that matters: an archive is unpacked somewhere else, and a
# binary that finds its libraries through an absolute path works in place and nowhere else.
check="$(mktemp -d)"
tar -xzf "$archive" -C "$check"
"$check/$name/cairn" --version >/dev/null
if [ "$(uname)" = Darwin ]; then
  outside="$(DYLD_PRINT_LIBRARIES=1 "$check/$name/cairn" --version 2>&1 |
    grep -c "/opt/homebrew\|/usr/local/Cellar" || true)"
  if [ "$outside" != "0" ]; then
    echo "  FAIL: $outside libraries still load from outside the archive" >&2
    rm -rf "$check"
    exit 1
  fi
fi
rm -rf "$check"
echo "  unpacked elsewhere and ran: ok"
