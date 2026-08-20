#!/usr/bin/env bash
#
# Make a macOS binary self-contained by copying the non-system dylibs it needs into the bundle
# and rewriting its load commands to point there.
#
# Why by hand rather than dylibbundler: otool and install_name_tool ship with the Command Line
# Tools, so this needs nothing installed. It also makes the two easily-missed steps explicit -
# the recursion, and the re-signing.
#
#   scripts/bundle_macos_dylibs.sh <binary> <frameworks-dir> [rpath-prefix]
#
# The Studio calls it from Tauri'''s beforeBundleCommand with the libraries going to
# src-tauri/resources/Frameworks and a prefix of @executable_path/../Resources/Frameworks:
# Tauri'''s bundle.macOS.frameworks only accepts a .framework or .dylib path, never a directory,
# so they travel as a named resource directory instead.
set -euo pipefail

BINARY="${1:?usage: bundle_macos_dylibs.sh <binary> <frameworks-dir> [rpath-prefix]}"
DEST="${2:?destination directory required}"
PREFIX="${3:-@executable_path/../Frameworks}"

if [ ! -f "$BINARY" ]; then
  echo "no such binary: $BINARY" >&2
  exit 1
fi
mkdir -p "$DEST"

# Anything under /usr/lib or /System is part of the OS and guaranteed present; everything else
# has to come along.
#
# Used for verification, where an @-prefixed entry is the desired outcome and not a problem.
external_deps() {
  otool -L "$1" | tail -n +2 | awk '{print $1}' \
    | grep -vE '^/usr/lib|^/System|^@' || true
}

# The run-paths a file searches for its @rpath dependencies.
rpaths_of() {
  otool -l "$1" | awk '/LC_RPATH/{f=1} f&&/ path /{print $2; f=0}'
}

# Dependencies to bundle, as absolute paths, following @rpath.
#
# Skipping @-prefixed entries here is the tempting mistake: Homebrew's libraries reference each
# other that way, so libgeos_c asks for @rpath/libgeos.dylib and the walk never learns libgeos
# exists. Everything looks bundled right up until the run-paths are cleaned and the loader has
# nowhere left to find it.
collect_deps() {
  local file="$1" dir dep base rpath candidate
  dir="$(cd "$(dirname "$file")" && pwd)"
  while IFS= read -r dep; do
    [ -z "$dep" ] && continue
    case "$dep" in
      /usr/lib/*|/System/*) continue ;;
      @rpath/*)
        base="${dep#@rpath/}"
        while IFS= read -r rpath; do
          case "$rpath" in
            @loader_path*) candidate="${dir}${rpath#@loader_path}/$base" ;;
            @executable_path*) candidate="${dir}${rpath#@executable_path}/$base" ;;
            *) candidate="$rpath/$base" ;;
          esac
          if [ -f "$candidate" ]; then
            echo "$candidate"
            break
          fi
        done < <(rpaths_of "$file")
        ;;
      @loader_path/*) [ -f "${dir}/${dep#@loader_path/}" ] && echo "${dir}/${dep#@loader_path/}" ;;
      @executable_path/*) [ -f "${dir}/${dep#@executable_path/}" ] && echo "${dir}/${dep#@executable_path/}" ;;
      *) echo "$dep" ;;
    esac
  done < <(otool -L "$file" | tail -n +2 | awk '{print $1}')
}

# Collect the transitive closure. The libraries themselves depend on each other - abseil alone is
# dozens of interlinked pieces - so stopping at the binary's direct dependencies leaves a bundle
# that still reaches outside for the second hop.
declare -a QUEUE=()
declare -A SEEN=()
while IFS= read -r dep; do
  [ -n "$dep" ] && QUEUE+=("$dep")
done < <(collect_deps "$BINARY")

declare -a LIBS=()
while [ ${#QUEUE[@]} -gt 0 ]; do
  current="${QUEUE[0]}"
  QUEUE=("${QUEUE[@]:1}")
  [ -n "${SEEN[$current]:-}" ] && continue
  SEEN[$current]=1
  if [ ! -f "$current" ]; then
    echo "warning: dependency not found on disk, skipping: $current" >&2
    continue
  fi
  LIBS+=("$current")
  while IFS= read -r dep; do
    [ -n "$dep" ] && [ -z "${SEEN[$dep]:-}" ] && QUEUE+=("$dep")
  done < <(collect_deps "$current")
done

echo "bundling ${#LIBS[@]} libraries into $DEST"

for lib in "${LIBS[@]}"; do
  name="$(basename "$lib")"
  if [ ! -f "$DEST/$name" ]; then
    cp -L "$lib" "$DEST/$name"
    chmod u+w "$DEST/$name"
  fi
done

# Rewrite: each copy gets an id under the prefix, and every reference in the binary and in the
# copies is repointed at it.
# The id is what a *new* dependent records when it links against the copy. `@rpath` keeps that
# independent of where in the bundle the dependent lives; each executable adds the run-path that
# resolves it.
for lib in "${LIBS[@]}"; do
  name="$(basename "$lib")"
  install_name_tool -id "@rpath/$name" "$DEST/$name" 2>/dev/null || true
done

# Rewrite by basename, driven by what the file actually references.
#
# Matching on the exact path collected earlier is not enough: Homebrew exposes the same library
# as both /opt/homebrew/opt/<pkg>/lib/x.dylib (a symlink) and
# /opt/homebrew/Cellar/<pkg>/<version>/lib/x.dylib, and different dependents record different
# spellings. A -change for one form leaves the other untouched, and the bundle then still
# reaches outside at runtime while otool on the main binary looks clean.
# The second argument is how the rewritten reference should be spelled, and it differs by who is
# doing the referencing:
#
#   the executable   $PREFIX, which is relative to *it*
#   a library        @loader_path, because the copies all sit in one directory together
#
# Using $PREFIX for library-to-library references ties the whole set to one executable's depth in
# the bundle. A second executable - valhalla_build_tiles, one directory deeper - then loads
# libssl, which asks for @executable_path/../Resources/Frameworks/libcrypto and lands nowhere.
retarget() {
  local target="$1" how="$2"
  local dep name
  while IFS= read -r dep; do
    [ -z "$dep" ] && continue
    case "$dep" in
      "$how"/*) continue ;;
    esac
    name="$(basename "$dep")"
    if [ -f "$DEST/$name" ]; then
      install_name_tool -change "$dep" "$how/$name" "$target" 2>/dev/null || true
    fi
  done < <(otool -L "$target" | tail -n +2 | awk '{print $1}' | grep -vE '^/usr/lib|^/System')
}

retarget "$BINARY" "$PREFIX"
for lib in "${LIBS[@]}"; do
  retarget "$DEST/$(basename "$lib")" "@loader_path"
done

# Run-path entries decide where @rpath dependencies are found, and they matter as much as the
# dependency list. Homebrew's libraries reference each other as @rpath/libfoo.dylib, and the
# binary inherits `-rpath /opt/homebrew/lib` from CMake's link line - so those resolve back to
# Homebrew no matter how carefully the load commands were rewritten. otool -L shows nothing
# wrong, because nothing is wrong there.
#
# Point the search at the bundle and take the outside entries away.
add_bundle_rpath() {
  local target="$1"
  install_name_tool -add_rpath "$PREFIX" "$target" 2>/dev/null || true
  local rpath
  while IFS= read -r rpath; do
    case "$rpath" in
      @*) continue ;;
      *) install_name_tool -delete_rpath "$rpath" "$target" 2>/dev/null || true ;;
    esac
  done < <(otool -l "$target" | awk '/LC_RPATH/{f=1} f&&/ path /{print $2; f=0}')
}

add_bundle_rpath "$BINARY"
for lib in "${LIBS[@]}"; do
  # @loader_path is already the Frameworks directory for a copied library
  install_name_tool -add_rpath "@loader_path" "$DEST/$(basename "$lib")" 2>/dev/null || true
  while IFS= read -r rpath; do
    case "$rpath" in
      @*) continue ;;
      *) install_name_tool -delete_rpath "$rpath" "$DEST/$(basename "$lib")" 2>/dev/null || true ;;
    esac
  done < <(otool -l "$DEST/$(basename "$lib")" | awk '/LC_RPATH/{f=1} f&&/ path /{print $2; f=0}')
done

# On Apple Silicon every Mach-O must carry a valid signature to load at all, and editing load
# commands invalidates whatever was there. Ad-hoc signing here is enough to make the bundle run;
# a release re-signs with the real identity afterwards, which is why this must happen *before*
# the signing step and not after it.
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
for lib in "${LIBS[@]}"; do
  codesign --force --sign "$IDENTITY" --timestamp=none "$DEST/$(basename "$lib")" 2>/dev/null || true
done
codesign --force --sign "$IDENTITY" --timestamp=none "$BINARY" 2>/dev/null || true

# Verify the *whole* bundle, not just the binary. Checking only the executable passes while a
# copied library still points at Homebrew for its own dependencies - which is a bundle that runs
# on the build machine and nowhere else.
failed=0
check() {
  local target="$1" left
  left="$(external_deps "$target")"
  if [ -n "$left" ]; then
    echo "error: $target still references:" >&2
    echo "$left" | sed 's/^/    /' >&2
    failed=1
  fi
}
# an absolute run-path is as much of a leak as an absolute dependency
check_rpaths() {
  local target="$1" outside
  outside="$(otool -l "$target" | awk '/LC_RPATH/{f=1} f&&/ path /{print $2; f=0}' | grep -v '^@' || true)"
  if [ -n "$outside" ]; then
    echo "error: $target keeps run-paths outside the bundle:" >&2
    echo "$outside" | sed 's/^/    /' >&2
    failed=1
  fi
}

# Every reference must actually resolve to a file, not merely look bundled.
#
# The checks above pass on a bundle that cannot start: a library referencing
# @executable_path/../Resources/Frameworks/libcrypto looks internal and is internal - to a
# different executable. Only following each reference to a real file catches that.
resolves() {
  local target="$1" exe_dir="$2" dep dir name candidate rpath ok
  dir="$(cd "$(dirname "$target")" && pwd)"
  while IFS= read -r dep; do
    [ -z "$dep" ] && continue
    case "$dep" in
      /usr/lib/*|/System/*) continue ;;
      # a reference through the prefix is resolved by where the app *will* be installed, which
      # does not exist yet at build time; what can be checked now is that the library it names
      # was actually collected
      "$PREFIX"/*)
        name="$(basename "$dep")"
        if [ ! -f "$DEST/$name" ]; then
          echo "error: $target -> $dep, but $name was not collected into $DEST" >&2
          failed=1
        fi
        continue
        ;;
      @loader_path/*) candidate="${dir}/${dep#@loader_path/}" ;;
      @executable_path/*) candidate="${exe_dir}/${dep#@executable_path/}" ;;
      @rpath/*)
        name="${dep#@rpath/}"
        ok=""
        while IFS= read -r rpath; do
          case "$rpath" in
            @loader_path*) candidate="${dir}${rpath#@loader_path}/$name" ;;
            @executable_path*) candidate="${exe_dir}${rpath#@executable_path}/$name" ;;
            *) candidate="$rpath/$name" ;;
          esac
          [ -f "$candidate" ] && { ok=1; break; }
        done < <(rpaths_of "$target")
        if [ -z "$ok" ]; then
          echo "error: $target -> $dep resolves to nothing" >&2
          failed=1
        fi
        continue
        ;;
      *) candidate="$dep" ;;
    esac
    if [ ! -f "$candidate" ]; then
      echo "error: $target -> $dep resolves to $candidate, which does not exist" >&2
      failed=1
    fi
  done < <(otool -L "$target" | tail -n +2 | awk '{print $1}')
}

BINARY_DIR="$(cd "$(dirname "$BINARY")" && pwd)"

check "$BINARY"
check_rpaths "$BINARY"
resolves "$BINARY" "$BINARY_DIR"
for lib in "${LIBS[@]}"; do
  check "$DEST/$(basename "$lib")"
  check_rpaths "$DEST/$(basename "$lib")"
  resolves "$DEST/$(basename "$lib")" "$BINARY_DIR"
done
if [ "$failed" != "0" ]; then
  exit 1
fi
echo "ok: $BINARY and ${#LIBS[@]} libraries resolve entirely inside the bundle"
