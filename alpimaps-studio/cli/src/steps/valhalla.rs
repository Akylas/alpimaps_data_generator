//! `alpimaps package` / `unpack` / `route` - the Valhalla steps.

use anyhow::{anyhow, Result};
use clap::Args as ClapArgs;
use std::path::PathBuf;
use studio_core::settings::Settings;
use studio_core::valhalla::package::{self, Compression, PackageOptions};
use studio_core::valhalla::routing;

#[derive(ClapArgs)]
pub struct PackageArgs {
    #[arg(long)]
    pub area: String,
    /// Valhalla tile directory. Defaults to <repo>/valhalla_tiles.
    #[arg(long)]
    pub tiles: Option<PathBuf>,
    /// Take the tile list from an existing package rather than a tilemask.
    #[arg(long)]
    pub like: Option<PathBuf>,
    /// zopfli is ~3% smaller than zlib and much slower; both emit ordinary gzip.
    #[arg(long, default_value = "zopfli")]
    pub compression: String,
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(ClapArgs)]
pub struct UnpackArgs {
    /// The .vtiles package to expand.
    pub package: PathBuf,
    /// Directory to write the tile tree into.
    pub tile_dir: PathBuf,
}

#[derive(ClapArgs)]
pub struct RouteArgs {
    /// A .vtiles package or a Valhalla tile directory.
    #[arg(long)]
    pub tiles: PathBuf,
    /// valhalla.json to use as the config template. Defaults to the configured one.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// lon,lat waypoints in order.
    #[arg(long = "point", value_name = "LON,LAT", required = true)]
    pub points: Vec<String>,
    #[arg(long, default_value = "auto")]
    pub costing: String,
    /// Print Valhalla's response as-is.
    #[arg(long)]
    pub json: bool,
}

pub fn package(settings: &Settings, args: PackageArgs) -> Result<()> {
    let tile_dir = args.tiles.unwrap_or_else(|| settings.repo_root.join("valhalla_tiles"));
    let output = args
        .output
        .unwrap_or_else(|| settings.area_dir(&args.area).join(format!("{}.vtiles", args.area)));

    // Selecting tiles from a .poly tilemask is still the Python script's job; taking the list
    // from an existing package covers rebuilding one, which is what this is mostly for.
    let reference = args.like.clone().unwrap_or_else(|| output.clone());
    let tiles = package::tiles_in(&reference).map_err(|_| {
        anyhow!(
            "no tile list: pass --like <existing.vtiles>, or generate one with \
             scripts/build_valhalla_package.py first"
        )
    })?;

    let compression = match args.compression.as_str() {
        "zopfli" => Compression::Zopfli,
        "zlib" => Compression::Zlib,
        other => return Err(anyhow!("unknown compression `{other}`")),
    };

    println!("packing {} tiles from {}", tiles.len(), tile_dir.display());
    let started = std::time::Instant::now();
    let report = package::build(
        &PackageOptions { package_id: args.area.clone(), tile_dir, output: output.clone(), compression },
        &tiles,
        |done, total| {
            if done % 10 == 0 || done == total {
                print!("\r  {done}/{total}   ");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        },
    )?;
    println!();

    println!(
        "wrote {} tiles in {:.1}s: {} raw -> {} gzip (ratio {:.3}), {} missing",
        report.tiles_written,
        started.elapsed().as_secs_f64(),
        super::mb(report.raw_bytes),
        super::mb(report.compressed_bytes),
        report.ratio(),
        report.tiles_missing
    );
    println!("  {}", output.display());
    Ok(())
}

pub fn unpack(args: UnpackArgs) -> Result<()> {
    let count = package::unpack(&args.package, &args.tile_dir)?;
    println!("unpacked {count} tiles into {}", args.tile_dir.display());
    Ok(())
}

pub fn route(settings: &Settings, args: RouteArgs) -> Result<()> {
    if !routing::available() {
        return Err(anyhow!(
            "this build has no Valhalla linked; rebuild with VALHALLA_DIR set"
        ));
    }
    let locations: Vec<[f64; 2]> = args
        .points
        .iter()
        .map(|raw| {
            let (lon, lat) = raw.split_once(',').ok_or_else(|| anyhow!("expected lon,lat: `{raw}`"))?;
            Ok([lon.trim().parse::<f64>()?, lat.trim().parse::<f64>()?])
        })
        .collect::<Result<_>>()?;

    // routing against an unpacked package exercises the artefact that ships, not the
    // intermediate tile directory
    let tile_dir = if args.tiles.is_file() {
        let out = std::env::temp_dir().join("alpimaps-vtiles");
        let _ = std::fs::remove_dir_all(&out);
        let count = package::unpack(&args.tiles, &out)?;
        println!("unpacked {count} tiles from {}", args.tiles.display());
        out
    } else {
        args.tiles.clone()
    };

    let template = args.config.unwrap_or_else(|| settings.valhalla_config_path());
    let mut router = routing::Router::open(&template, &tile_dir)?;
    let request = routing::RouteRequest { locations, costing: args.costing.clone() };
    let response = router.route(&request.to_json())?;

    if args.json {
        println!("{response}");
        return Ok(());
    }
    let parsed: serde_json::Value = serde_json::from_str(&response)?;
    let summary = &parsed["trip"]["summary"];
    println!(
        "{:.1} km, {:.0} min, {} costing",
        summary["length"].as_f64().unwrap_or(0.0),
        summary["time"].as_f64().unwrap_or(0.0) / 60.0,
        args.costing
    );
    for leg in parsed["trip"]["legs"].as_array().unwrap_or(&vec![]) {
        for m in leg["maneuvers"].as_array().unwrap_or(&vec![]) {
            println!(
                "  {:>6.2} km  {}",
                m["length"].as_f64().unwrap_or(0.0),
                m["instruction"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}
