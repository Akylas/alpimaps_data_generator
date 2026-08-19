//! `alpimaps profile` - elevation along a line.

use anyhow::{anyhow, Result};
use clap::Args as ClapArgs;
use std::path::PathBuf;
use studio_core::elevation::TerrainSampler;

#[derive(ClapArgs)]
pub struct Args {
    /// Terrain-RGB archive to sample.
    #[arg(long)]
    pub terrain: PathBuf,
    /// lon,lat pairs, repeatable and in order: --point 5.7,45.2 --point 6.8,45.8
    #[arg(long = "point", value_name = "LON,LAT", required = true)]
    pub points: Vec<String>,
    /// Zoom to sample at. Defaults to the archive's maximum.
    #[arg(long)]
    pub zoom: Option<u8>,
    /// Spacing of intermediate samples, in metres.
    #[arg(long, default_value_t = 50.0)]
    pub densify: f64,
    /// Hysteresis for ascent and descent. Zero turns quantisation dither into phantom climb.
    #[arg(long, default_value_t = 3.0)]
    pub threshold: f64,
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: Args) -> Result<()> {
    let line: Vec<[f64; 2]> = args
        .points
        .iter()
        .map(|raw| {
            let (lon, lat) = raw.split_once(',').ok_or_else(|| anyhow!("expected lon,lat: `{raw}`"))?;
            Ok([lon.trim().parse::<f64>()?, lat.trim().parse::<f64>()?])
        })
        .collect::<Result<_>>()?;

    let mut sampler = TerrainSampler::open(&args.terrain)?;
    let zoom = args.zoom.unwrap_or(sampler.maxzoom);
    let profile = sampler.profile(&line, zoom, args.densify, args.threshold)?;

    if args.json {
        println!("{}", serde_json::to_string(&profile)?);
        return Ok(());
    }
    println!(
        "{:.2} km, {} samples at z{}, {:.0}-{:.0} m, +{:.0} / -{:.0} m (threshold {} m), {} gaps",
        profile.distance_m / 1000.0,
        profile.points.len(),
        profile.zoom,
        profile.min_m.unwrap_or(0.0),
        profile.max_m.unwrap_or(0.0),
        profile.ascent_m,
        profile.descent_m,
        profile.threshold_m,
        profile.gaps
    );
    Ok(())
}
