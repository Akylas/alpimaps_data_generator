//! The CLI links studio-core, which links Valhalla, so it has to repeat its link arguments:
//! Cargo's link directives do not propagate to dependent crates.
include!("../valhalla-link.rs");

fn main() {
    println!("cargo:rustc-check-cfg=cfg(valhalla)");
    if link_valhalla() {
        println!("cargo:rustc-cfg=valhalla");
    }
}
