//! Optional native link against Valhalla.
//!
//! Valhalla is a large CMake project whose link line runs to a hundred dylibs, so rather than
//! restating it this reads the one CMake already produced. `link.txt` and `flags.make` in a
//! built tree carry the exact flags that tree needs, which keeps this working across Homebrew
//! upgrades without edits here.
//!
//! Entirely opt-in: without a built Valhalla the crate compiles as before, and routing simply
//! reports itself unavailable.

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
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Valhalla's headers use `std::filesystem::path`, which libc++ marks unavailable before
        // 10.15. cc-rs otherwise defaults to 10.13 - and `tauri build` sets that explicitly -
        // so the shim fails to compile in a release bundle while building fine in dev.
        cc.flag("-mmacosx-version-min=11.0");
    }
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

    emit_link_args(&link_line, &build);
    println!("cargo:rustc-cfg=valhalla");
}


include!("../valhalla-link.rs");
