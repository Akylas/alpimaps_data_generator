//! Optional native link against Valhalla.
//!
//! Valhalla is a large CMake project whose link line runs to a hundred dylibs, so rather than
//! restating it this reads the one CMake already produced. `link.txt` and `flags.make` in a
//! built tree carry the exact flags that tree needs, which keeps this working across Homebrew
//! upgrades without edits here.
//!
//! Entirely opt-in: without a built Valhalla the crate compiles as before, and routing simply
//! reports itself unavailable.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=VALHALLA_DIR");
    println!("cargo:rustc-check-cfg=cfg(valhalla)");

    let Some(root) = valhalla_root() else {
        println!("cargo:warning=valhalla not found; routing disabled (set VALHALLA_DIR)");
        return;
    };
    let build = root.join("build");
    if !build.join("src/libvalhalla.a").is_file() {
        println!("cargo:warning=no libvalhalla.a under {}; routing disabled", build.display());
        return;
    }

    let Some(link_line) = read_link_line(&build) else {
        println!("cargo:warning=no CMake link.txt in {}; routing disabled", build.display());
        return;
    };
    let includes = read_includes(&build).unwrap_or_default();

    println!("cargo:rerun-if-changed=src-cpp/valhalla_shim.cc");
    let mut cc = cc::Build::new();
    cc.cpp(true).file("src-cpp/valhalla_shim.cc").std("gnu++20").opt_level(2);
    for dir in includes {
        cc.include(dir);
    }
    // Valhalla's own definitions; the boost ones in particular change header behaviour, so the
    // shim has to be compiled with the same view of boost as the library it calls into
    for define in [
        "BOOST_ALLOW_DEPRECATED_HEADERS",
        "BOOST_BIND_GLOBAL_PLACEHOLDERS",
        "BOOST_NO_CXX11_SCOPED_ENUMS",
        "PROTOBUF_USE_DLLS",
    ] {
        cc.define(define, None);
    }
    cc.compile("valhalla_shim");

    for arg in link_args(&link_line, &build) {
        println!("cargo:rustc-link-arg={arg}");
    }
    println!("cargo:rustc-cfg=valhalla");
}

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
            t if t.ends_with(".dylib") || t.ends_with(".a") || t.ends_with(".so") => {
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
    // libvalhalla always needs these two and CMake resolves them through the compiler's own
    // defaults rather than naming them on the link line
    args.push("-lcurl".into());
    args.push("-lz".into());
    args.push("-lc++".into());
    args
}
