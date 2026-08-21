#!/usr/bin/env bash
#
# Copy the libraries a Linux binary needs next to it, and point the binary at them.
#
#   scripts/bundle_linux_libs.sh <binary> <libdir> [rpath]
#
# The Linux counterpart of bundle_macos_dylibs.sh, and it exists for the same reason plus one
# more. On macOS the libraries come from Homebrew and are simply absent elsewhere. On Linux they
# are present everywhere - under different names:
#
#   Ubuntu 22.04   libprotobuf.so.23   libspatialite.so.7
#   Ubuntu 24.04   libprotobuf.so.32   libspatialite.so.8
#
# A soname is part of the link, so a binary built against one release cannot load the other's
# library at all. Carrying them is what lets a single build run on both.
#
# The C runtime is deliberately *not* carried: glibc is not relocatable that way, and a newer one
# than the build machine's is not needed. That is what sets the floor - build on the oldest
# release you mean to support. libstdc++ is left out too, because the binaries this packages are
# linked with -static-libstdc++; see the workflow.
set -euo pipefail

BIN="${1:?usage: bundle_linux_libs.sh <binary> <libdir> [rpath]}"
LIBDIR="${2:?usage: bundle_linux_libs.sh <binary> <libdir> [rpath]}"
RPATH="${3:-\$ORIGIN/lib}"

command -v patchelf >/dev/null || { echo "patchelf not installed" >&2; exit 1; }

mkdir -p "$LIBDIR"

# Left to the system: the C runtime and the pieces tied to it, plus libstdc++ (static in these
# binaries) and the graphics and desktop stack, which has to be the one the session is running.
skip() {
  case "$1" in
    libc.so.*|libm.so.*|libpthread.so.*|libdl.so.*|librt.so.*|ld-linux*|libld-linux*) return 0 ;;
    libgcc_s.so.*|libstdc++.so.*) return 0 ;;
    libGL*|libEGL*|libGLX*|libX11*|libxcb*|libwayland*|libdrm*|libgbm*) return 0 ;;
    libgtk-*|libgdk-*|libwebkit2gtk*|libjavascriptcoregtk*|libsoup*|libgio-*|libglib-*|libgobject-*) return 0 ;;
    *) return 1 ;;
  esac
}

carried=0
while IFS= read -r line; do
  path="$(printf '%s' "$line" | awk '/=> \//{print $3}')"
  [ -n "$path" ] || continue
  name="$(basename "$path")"
  skip "$name" && continue
  [ -f "$LIBDIR/$name" ] && continue
  cp -L "$path" "$LIBDIR/$name"
  chmod u+w "$LIBDIR/$name"
  carried=$((carried + 1))
done < <(ldd "$BIN")

# The libraries carried have dependencies of their own, and those have to come along too -
# otherwise the first one that needs a sibling fails to load on a machine that has neither.
# Repeats until a pass adds nothing.
while :; do
  added=0
  for lib in "$LIBDIR"/*; do
    [ -f "$lib" ] || continue
    while IFS= read -r line; do
      path="$(printf '%s' "$line" | awk '/=> \//{print $3}')"
      [ -n "$path" ] || continue
      name="$(basename "$path")"
      skip "$name" && continue
      [ -f "$LIBDIR/$name" ] && continue
      cp -L "$path" "$LIBDIR/$name"
      chmod u+w "$LIBDIR/$name"
      added=$((added + 1))
      carried=$((carried + 1))
    done < <(ldd "$lib" 2>/dev/null)
  done
  [ "$added" = 0 ] && break
done

# $ORIGIN is resolved by the loader against the file being loaded, so the libraries get their own
# rpath pointing at the directory they are already in - a library moved with the binary must find
# its siblings without depending on where the binary sits relative to them.
patchelf --set-rpath "$RPATH" "$BIN"
for lib in "$LIBDIR"/*; do
  [ -f "$lib" ] || continue
  patchelf --set-rpath '$ORIGIN' "$lib" 2>/dev/null || true
done

echo "$carried"
