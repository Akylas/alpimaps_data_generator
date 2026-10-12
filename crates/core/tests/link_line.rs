//! The CMake link line is parsed by a build script, which no test normally reaches - and the one
//! thing it got wrong was invisible on the platform it was written on.
//!
//! `ends_with(".so")` matches nothing on Linux, where the link line carries the versioned soname:
//!
//!   /opt/protobuf/lib/libprotobuf.so.3.21.12.0
//!
//! Dropped silently, that produced `undefined symbol:
//! google::protobuf::internal::fixed_address_empty_string` at the end of a CI build, while macOS
//! kept working because Homebrew's path really does end in `.dylib`.
#![allow(dead_code)]

include!("../../valhalla-link.rs");

#[test]
fn versioned_sonames_are_libraries() {
    for token in [
        "/opt/protobuf/lib/libprotobuf.so.3.21.12.0",
        "/usr/lib/x86_64-linux-gnu/libspatialite.so.7",
        "/usr/lib/x86_64-linux-gnu/libgeos_c.so.1",
        "/usr/lib/x86_64-linux-gnu/libluajit-5.1.so",
    ] {
        assert!(is_library_path(token), "dropped from the link line: {token}");
    }
}

#[test]
fn macos_and_static_forms_are_libraries() {
    for token in ["/opt/homebrew/lib/libprotobuf-lite.35.1.0.dylib", "src/libvalhalla.a"] {
        assert!(is_library_path(token), "dropped from the link line: {token}");
    }
}

#[test]
fn flags_and_plain_words_are_not() {
    for token in ["-lcurl", "-L/usr/lib", "-Wl,-search_paths_first", "-framework", "CoreFoundation"]
    {
        assert!(!is_library_path(token), "mistaken for a library: {token}");
    }
    // the directory is what carries .so here, not the file
    assert!(!is_library_path("/some/dir.so.d/notalib"));
}

#[test]
fn a_linux_link_line_keeps_its_libraries() {
    let line = "/usr/bin/c++ -O3 CMakeFiles/x.dir/a.cc.o -o valhalla_build_tiles \
                src/libvalhalla.a /opt/protobuf/lib/libprotobuf.so.3.21.12.0 \
                /usr/lib/x86_64-linux-gnu/libspatialite.so.7 -lm";
    let args = link_args(line, Path::new("/build"));
    for needle in ["libvalhalla.a", "libprotobuf.so.3.21.12.0", "libspatialite.so.7", "-lm"] {
        assert!(args.iter().any(|a| a.contains(needle)), "{needle} missing from {args:?}");
    }
}

#[test]
fn the_cxx_runtime_matches_the_platform() {
    let args = link_args("/usr/bin/c++ -o x a.o", Path::new("/build"));
    let expected = if cfg!(target_os = "macos") { "-lc++" } else { "-lstdc++" };
    assert!(args.iter().any(|a| a == expected), "expected {expected} in {args:?}");
}
