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
else
  # Linux carries libraries for a different reason: they exist on every distribution, under
  # names that change between releases (libprotobuf.so.23 on 22.04, .32 on 24.04). Carrying
  # them is what lets one build run on both.
  carried="$("$repo/scripts/bundle_linux_libs.sh" "$stage/cairn" "$stage/lib" '$ORIGIN/lib')"
  echo "  carried $carried libraries into lib/"
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

lib/ holds the libraries this needs that differ between distributions - protobuf, spatialite,
geos, luajit and their dependencies. Keep it next to the binary; `cairn` looks for it there.

Built on Ubuntu 22.04, so it needs glibc 2.35 or newer: Ubuntu 22.04 and later, Debian 12 and
later, or anything of that vintage. Nothing to install.
LINUX
fi)
EOF

archive="$OUT/${name}.tar.gz"
tar -czf "$archive" -C "$OUT" "$name"
# shasum is the macOS spelling, sha256sum the GNU one - and a minimal Linux image has only the
# second, so neither can be assumed
if command -v sha256sum >/dev/null; then
  ( cd "$OUT" && sha256sum "${name}.tar.gz" > "${name}.tar.gz.sha256" )
else
  ( cd "$OUT" && shasum -a 256 "${name}.tar.gz" > "${name}.tar.gz.sha256" )
fi

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
else
  # Every library that was carried has to be the one that actually loads. The build machine has
  # system copies of most of them, so an rpath that failed to take would look fine here and fail
  # on a machine of a different vintage - which is the whole point of carrying them.
  #
  # Read once into a variable rather than piped per library: `grep -q` stops at the first match
  # and leaves ldd with a SIGPIPE, which `set -o pipefail` then reports as a failed pipeline -
  # so a matching library would be read as a missing one.
  deps="$(ldd "$check/$name/cairn" "$check/$name/lib"/* 2>/dev/null || true)"
  for lib in "$check/$name/lib"/*; do
    [ -f "$lib" ] || continue
    soname="$(basename "$lib")"
    case "$deps" in
      *"$soname => $check/$name/lib/$soname"*) ;;
      *)
        echo "  FAIL: $soname is carried but the system copy is what loads" >&2
        rm -rf "$check"
        exit 1
        ;;
    esac
  done
  echo "  $(ls -1 "$check/$name/lib" | wc -l | tr -d ' ') carried libraries all load from the archive"
  # libstdc++ is static in `cairn` itself; if it shows up as a dependency there, the link flags
  # were lost and the binary will not start on a release older than the one it was built on.
  #
  # Only the binary: the carried libraries come from the distribution and link the system C++
  # runtime, which is correct - they were built against an older one than this binary needs.
  # ldd walks the whole closure, and the carried libraries pull libstdc++ in themselves, so it
  # cannot answer this. The direct NEEDED entries can.
  own="$(patchelf --print-needed "$check/$name/cairn" 2>/dev/null || true)"
  case "$own" in
    *libstdc++*)
      echo "  FAIL: links libstdc++ dynamically - it will not run on an older release" >&2
      rm -rf "$check"
      exit 1
      ;;
  esac
fi
rm -rf "$check"
echo "  unpacked elsewhere and ran: ok"
