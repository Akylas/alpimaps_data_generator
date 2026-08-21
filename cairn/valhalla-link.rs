// Shared build-script logic for linking Valhalla.
//
// `include!`d rather than shared as a crate because Cargo link directives do not propagate:
// `cargo:rustc-link-arg` applies only to the crate that emits it, so every binary that links
// studio-core has to emit the same arguments itself or the link fails on undefined symbols.
// The alternative - a build-dependency crate - is more machinery for the same twenty lines.

use std::path::{Path, PathBuf};

fn valhalla_root() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("VALHALLA_DIR") {
        let path = PathBuf::from(dir);
        return path.is_dir().then_some(path);
    }
    // the sibling checkout this repository builds
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    let guess = manifest.join("../../valhalla");
    guess.is_dir().then(|| guess.canonicalize().unwrap_or(guess))
}

/// CMake writes one `link.txt` per target and they are not all equivalent - a target that never
/// touches the tile downloader omits curl, and one that never reads compressed tiles omits zlib.
/// Prefer a tool known to exercise the whole library, and fall back to any of them.
fn read_link_line(build: &Path) -> Option<String> {
    for target in ["valhalla_build_tiles", "valhalla_service", "valhalla_export_edges"] {
        let path = build.join("CMakeFiles").join(format!("{target}.dir")).join("link.txt");
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Some(text);
        }
    }
    read_first(build, "link.txt")
}

fn read_first(build: &Path, name: &str) -> Option<String> {
    fn walk(dir: &Path, name: &str, depth: usize, found: &mut Option<PathBuf>) {
        if found.is_some() || depth > 4 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, name, depth + 1, found);
            } else if path.file_name().is_some_and(|f| f == name) {
                *found = Some(path);
                return;
            }
        }
    }
    let mut found = None;
    walk(&build.join("CMakeFiles"), name, 0, &mut found);
    std::fs::read_to_string(found?).ok()
}

/// Include directories, taken from any object's compile flags so third-party header paths come
/// along too.
fn read_includes(build: &Path) -> Option<Vec<PathBuf>> {
    let flags = read_first(build, "flags.make")?;
    let line = flags.lines().find(|l| l.starts_with("CXX_INCLUDES"))?;
    let mut dirs = Vec::new();
    let mut tokens = line.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        if let Some(rest) = token.strip_prefix("-I") {
            dirs.push(PathBuf::from(rest));
        } else if token == "-isystem" {
            if let Some(next) = tokens.next() {
                dirs.push(PathBuf::from(next));
            }
        }
    }
    Some(dirs)
}

/// Turn CMake's link line into linker arguments.
///
/// Only the library and search-path parts are kept: the compiler driver, the source objects and
/// the output flag belong to CMake's own invocation and would confuse Rust's.
fn link_args(link_line: &str, build: &Path) -> Vec<String> {
    let mut args = Vec::new();
    let mut tokens = link_line.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        match token {
            // `rustc-link-arg` goes to the compiler driver, not the linker, so the `-Wl,`
            // prefix has to stay: stripping it turns `-Wl,-search_paths_first` into an
            // argument clang itself does not know.
            t if t.starts_with("-Wl,") => args.push(t.to_string()),
            t if t.starts_with("-L") || t.starts_with("-l") || t.starts_with("-F") => {
                args.push(t.to_string())
            }
            t if is_library_path(t) => {
                // CMake writes these relative to the build directory
                let path = Path::new(t);
                let resolved = if path.is_absolute() { path.to_path_buf() } else { build.join(path) };
                args.push(resolved.display().to_string());
            }
            "-framework" => {
                if let Some(name) = tokens.next() {
                    args.push("-framework".into());
                    args.push(name.to_string());
                }
            }
            _ => {}
        }
    }
    // libvalhalla always needs these and CMake resolves them through the compiler's own defaults
    // rather than naming them on the link line. The C++ runtime is whichever one the platform's
    // compiler uses - libc++ for clang on macOS, libstdc++ for GCC elsewhere. Naming the wrong
    // one leaves every C++ symbol in libvalhalla undefined.
    args.push("-lcurl".into());
    args.push("-lz".into());
    args.push(if cfg!(target_os = "macos") { "-lc++".into() } else { "-lstdc++".to_string() });
    args
}

/// Whether a link-line token names a library file rather than a flag.
///
/// Not `ends_with(".so")`: a Linux link line carries the versioned soname, and that is the
/// normal form, not an edge case -
///
///   /opt/protobuf/lib/libprotobuf.so.3.21.12.0
///   /usr/lib/x86_64-linux-gnu/libspatialite.so.7
///
/// which an extension test drops without saying so. What follows is a link that succeeds locally
/// against Homebrew - where CMake writes libprotobuf-lite.35.1.0.dylib, and .dylib *is* the
/// suffix - and fails on Linux with undefined symbols from every library that was skipped.
fn is_library_path(token: &str) -> bool {
    if token.starts_with('-') {
        return false;
    }
    let Some(name) = Path::new(token).file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    // the version follows .so, so it is a contains rather than a suffix; taking the file name
    // first keeps a directory called something.so.d from counting
    name.contains(".so") || name.ends_with(".dylib") || name.ends_with(".a") || name.ends_with(".tbd")
}

/// Emit the link arguments for a crate that links Valhalla.
pub fn emit_link_args(link_line: &str, build: &Path) {
    for arg in link_args(link_line, build) {
        println!("cargo:rustc-link-arg={arg}");
    }
}

/// Everything a binary crate needs: find the tree, read its link line, emit the arguments.
///
/// Returns false when no built Valhalla is present, which is not an error - the app builds
/// without it and reports routing unavailable.
pub fn link_valhalla() -> bool {
    println!("cargo:rerun-if-env-changed=VALHALLA_DIR");
    let Some(root) = valhalla_root() else { return false };
    let build = root.join("build");
    if !build.join("src/libvalhalla.a").is_file() {
        return false;
    }
    let Some(link_line) = read_link_line(&build) else { return false };
    emit_link_args(&link_line, &build);
    true
}
