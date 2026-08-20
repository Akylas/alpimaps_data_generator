//! `cairn catalog` - what has been generated, and how big it is.

use super::mb;
use anyhow::Result;
use clap::Args as ClapArgs;
use cairn_core::catalog;
use cairn_core::settings::Settings;

#[derive(ClapArgs)]
pub struct Args {
    /// Only this area.
    #[arg(long)]
    pub area: Option<String>,
    /// Count tiles and bytes per zoom. Walks the whole archive, so it is the slow option.
    #[arg(long)]
    pub stats: bool,
    /// Machine-readable output.
    #[arg(long)]
    pub json: bool,
}

pub fn run(settings: &Settings, args: Args) -> Result<()> {
    let mut areas = catalog::discover(&settings.output_root)?;
    if let Some(wanted) = &args.area {
        areas.retain(|a| &a.name == wanted);
    }
    if areas.is_empty() {
        println!("nothing under {}", settings.output_root.display());
        return Ok(());
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&areas)?);
        return Ok(());
    }

    for area in &areas {
        println!("\n{} ({})", area.name, mb(area.total_bytes()));
        for art in &area.artifacts {
            print!("  {:<38} {:>10}  {:?}", art.file_name, mb(art.size_bytes), art.kind);
            if let Some(variant) = &art.variant {
                print!(" [{variant}]");
            }
            if let Some(encoding) = &art.encoding {
                print!(" {encoding}");
            }
            if let (Some(min), Some(max)) = (art.minzoom, art.maxzoom) {
                print!(" z{min}-{max}");
            }
            if let Some(error) = &art.probe_error {
                print!("  UNREADABLE: {error}");
            }
            println!();

            if args.stats && art.probe_error.is_none() {
                match catalog::tile_stats(&art.path) {
                    Ok(s) => {
                        // addressed and unique differ under --compact-db, where one blob can
                        // serve many tiles; reporting only one of them misleads either way
                        print!(
                            "    {} tiles / {} addressed",
                            s.addressed_tiles,
                            mb(s.addressed_bytes)
                        );
                        if let (Some(tiles), Some(bytes)) = (s.unique_tiles, s.unique_bytes) {
                            print!("   {tiles} / {} unique ({:.1}% deduplicated)", mb(bytes), s.dedup_ratio() * 100.0);
                        }
                        println!();
                        let per_zoom: Vec<String> = s
                            .per_zoom
                            .iter()
                            .rev()
                            .take(5)
                            .map(|z| format!("z{}={}", z.zoom, mb(z.bytes)))
                            .collect();
                        if !per_zoom.is_empty() {
                            println!("    {}", per_zoom.join("  "));
                        }
                    }
                    Err(e) => println!("    stats failed: {e}"),
                }
            }
        }
    }
    Ok(())
}
