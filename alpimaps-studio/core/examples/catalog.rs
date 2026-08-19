//! Print the catalog for a real output root.
//!
//!   cargo run -p studio-core --example catalog -- ../alpimaps_mbtiles [--stats]

use studio_core::catalog;

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("usage: catalog <output_root> [--stats]"));
    let with_stats = args.any(|a| a == "--stats");

    for area in catalog::discover(&root)? {
        println!("\n=== {} ({}) ===", area.name, mb(area.total_bytes()));
        for art in &area.artifacts {
            print!(
                "{:<38} {:>10}  {:?}/{:?}",
                art.file_name,
                mb(art.size_bytes),
                art.kind,
                art.format
            );
            if let Some(v) = &art.variant {
                print!("  variant={v}");
            }
            if let Some(e) = &art.encoding {
                print!("  encoding={e}");
            }
            if let (Some(a), Some(b)) = (art.minzoom, art.maxzoom) {
                print!("  z{a}-{b}");
            }
            if !art.layers.is_empty() {
                print!("  layers={}", art.layers.len());
            }
            if let Some(h) = &art.provenance.githash {
                print!("  git={}", &h[..7.min(h.len())]);
            }
            if let Some(e) = &art.probe_error {
                print!("  ERROR: {e}");
            }
            println!();

            if with_stats && art.probe_error.is_none() {
                match catalog::tile_stats(&art.path) {
                    Ok(s) => {
                        println!(
                            "  addressed {} tiles / {}   unique {} / {}   dedup {:.1}%",
                            s.addressed_tiles,
                            mb(s.addressed_bytes),
                            s.unique_tiles.map_or("-".into(), |v| v.to_string()),
                            s.unique_bytes.map_or("-".into(), mb),
                            s.dedup_ratio() * 100.0
                        );
                        let top: Vec<String> = s
                            .per_zoom
                            .iter()
                            .rev()
                            .take(4)
                            .map(|z| format!("z{}={}", z.zoom, mb(z.bytes)))
                            .collect();
                        println!("  {}", top.join("  "));
                    }
                    Err(e) => println!("  stats failed: {e}"),
                }
            }
        }
    }
    Ok(())
}
