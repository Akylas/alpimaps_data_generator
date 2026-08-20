//! Rebuild a routing package from a Valhalla tile directory and compare it against the one the
//! Python script produced.
//!
//!   cargo run --release -p cairn-core --example vpackage -- \
//!       ../valhalla_tiles ../alpimaps_mbtiles/rhone-alpes/rhone-alpes.vtiles [--limit N] [--zlib]

use cairn_core::valhalla::package::{self, Compression, PackageOptions};

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let tile_dir = std::path::PathBuf::from(args.next().expect("usage: vpackage <tile_dir> <reference.vtiles> [--limit N] [--zlib]"));
    let reference = std::path::PathBuf::from(args.next().expect("reference .vtiles required"));
    let rest: Vec<String> = args.collect();
    let limit = rest
        .iter()
        .position(|a| a == "--limit")
        .and_then(|i| rest.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok());
    let how = if rest.iter().any(|a| a == "--zlib") { Compression::Zlib } else { Compression::Zopfli };

    // take the exact tile list the existing package carries, so path building is under test
    let mut tiles = package::tiles_in(&reference)?;
    println!("reference lists {} tiles", tiles.len());

    let (found, missing) = package::resolve(&tile_dir, &tiles);
    println!(
        "path resolution: {} of {} found on disk, {} missing",
        found.len(),
        tiles.len(),
        missing.len()
    );
    for tile in missing.iter().take(5) {
        println!("  missing {:?} -> {}", tile, tile.path());
    }
    if !missing.is_empty() {
        println!("  (a miss means the id-to-path mapping disagrees with the Python original)");
    }

    if let Some(n) = limit {
        tiles.truncate(n);
        println!("limited to {n} tiles");
    }

    let out = std::env::temp_dir().join("rust-package.vtiles");
    let opts = PackageOptions {
        package_id: "rhone-alpes".into(),
        tile_dir,
        output: out.clone(),
        compression: how,
    };
    println!("compressing with {how:?}…");
    let started = std::time::Instant::now();
    let report = package::build(&opts, &tiles, |done, total| {
        if done % 25 == 0 || done == total {
            print!("\r  {done}/{total}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
    })?;
    println!();

    let produced = std::fs::metadata(&out)?.len();
    println!(
        "\nwrote {} tiles in {:.1}s: raw {} -> gzip {} (ratio {:.3})",
        report.tiles_written,
        started.elapsed().as_secs_f64(),
        mib(report.raw_bytes),
        mib(report.compressed_bytes),
        report.ratio()
    );
    println!("file on disk: {}", mib(produced));

    if limit.is_none() {
        let expected = std::fs::metadata(&reference)?.len();
        let delta = produced as f64 / expected as f64 - 1.0;
        println!(
            "reference:    {}  ({:+.2}% versus the Python package)",
            mib(expected),
            delta * 100.0
        );
    }
    Ok(())
}
