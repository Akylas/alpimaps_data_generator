//! Same as the CLI: linking studio-core means repeating its Valhalla link arguments, because
//! Cargo link directives stop at the crate that emits them.
include!("../valhalla-link.rs");

fn main() {
    println!("cargo:rustc-check-cfg=cfg(valhalla)");
    if link_valhalla() {
        println!("cargo:rustc-cfg=valhalla");
    }
    tauri_build::build()
}
