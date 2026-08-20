#!/usr/bin/env python3
"""Drop the native payloads a fat jar carries for platforms this build will never run on.

planetiler's `-with-deps.jar` is 89 MB, and only about 1.7 MB of that is planetiler. The rest is
its dependency tree, and the largest single slice is native code shipped for every platform at
once: SQLite alone carries builds for Linux, Windows, macOS, FreeBSD and several architectures.
An app bundle for one platform needs one of them.

Only files under a *native library root* are considered, and only when their path names a
platform. Class files are never touched, so a package that happens to be called
`org.something.linux` is not at risk - which is why this matches on directory position rather
than on the string appearing anywhere.

    strip_jar_natives.py <in.jar> <out.jar> [--os macos|linux|windows] [--arch aarch64|x86_64]

Prints what it removed and what it kept.
"""

import argparse
import platform
import re
import shutil
import sys
import zipfile
from pathlib import Path

# Where the libraries actually live. Anything outside these is left alone.
NATIVE_ROOTS = (
    "org/sqlite/native/",
    "org/xerial/snappy/native/",
    "META-INF/native/",
    "native/",
    "com/sun/jna/",
    "org/lwjgl/",
    "io/netty/resources/",
)

# How each platform spells itself in those paths.
OS_WORDS = {
    "macos": ("mac", "macos", "macosx", "osx", "darwin"),
    "linux": ("linux",),
    "windows": ("windows", "win32", "win"),
    "freebsd": ("freebsd",),
    "aix": ("aix",),
    "sunos": ("sunos", "solaris"),
}

ARCH_WORDS = {
    "aarch64": ("aarch64", "arm64"),
    "x86_64": ("x86_64", "x86-64", "amd64", "x64"),
    "x86": ("x86", "i386", "i686"),
    "armv7": ("armv7", "arm"),
    "ppc64": ("ppc64", "ppc64le"),
    "s390x": ("s390x",),
}

# Native library file names. A jar entry that is not one of these is data, and data is kept.
NATIVE_SUFFIXES = (".so", ".dylib", ".dll", ".jnilib", ".a")


def classify(path: str):
    """`(os, arch)` this entry is for, as far as its path says. `None` means it does not say."""
    if not any(path.startswith(root) for root in NATIVE_ROOTS):
        return None, None
    if not path.endswith(NATIVE_SUFFIXES) and ".so." not in path:
        return None, None

    # match on whole path segments: `Linux/` is a platform, `linuxes.txt` is not
    segments = [s.lower() for s in re.split(r"[/\-_.]", path) if s]
    found_os = next((name for name, words in OS_WORDS.items() if set(words) & set(segments)), None)
    found_arch = next(
        (name for name, words in ARCH_WORDS.items() if set(words) & set(segments)), None
    )
    return found_os, found_arch


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("source")
    parser.add_argument("target")
    parser.add_argument("--os", default=None)
    parser.add_argument("--arch", default=None)
    args = parser.parse_args()

    host_os = args.os or {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(
        platform.system(), "linux"
    )
    host_arch = args.arch or {"arm64": "aarch64", "x86_64": "x86_64"}.get(
        platform.machine(), platform.machine()
    )

    source, target = Path(args.source), Path(args.target)
    dropped_bytes = 0
    dropped = 0

    with zipfile.ZipFile(source) as zin:
        entries = zin.infolist()
        keep = []
        for entry in entries:
            entry_os, entry_arch = classify(entry.filename)
            # only drop when the entry names a platform *and* it is not ours; an unlabelled
            # native library is kept, because guessing wrong breaks the build silently
            if entry_os is not None and entry_os != host_os:
                dropped += 1
                dropped_bytes += entry.compress_size
                continue
            if entry_arch is not None and entry_os == host_os and entry_arch != host_arch:
                dropped += 1
                dropped_bytes += entry.compress_size
                continue
            keep.append(entry)

        if not keep:
            print("refusing to write an empty jar", file=sys.stderr)
            return 1

        # copy through rather than rebuild: compression settings and the manifest stay as they
        # were, so what runs is the same jar minus the entries removed
        with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as zout:
            for entry in keep:
                zout.writestr(entry, zin.read(entry.filename))

    before = source.stat().st_size
    after = target.stat().st_size
    print(
        f"  {before / 1048576:.1f} MB -> {after / 1048576:.1f} MB "
        f"({dropped} entries for other platforms, {dropped_bytes / 1048576:.1f} MB compressed)"
    )
    if after >= before:
        # nothing gained; keep the original rather than a repacked copy of it
        shutil.copyfile(source, target)
    return 0


if __name__ == "__main__":
    sys.exit(main())
