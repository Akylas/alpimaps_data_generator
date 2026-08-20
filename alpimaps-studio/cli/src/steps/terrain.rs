//! `alpimaps terrain` - terrain-RGB tiles, mirroring build_terrain_rgb.py's flags.

use anyhow::{anyhow, Result};
use clap::Args as ClapArgs;
use std::path::PathBuf;
use studio_core::elevation::Encoding;
use studio_core::settings::Settings;
use studio_core::steps::{state, StepId};
use studio_core::terrain::{render, source};

#[derive(ClapArgs)]
pub struct Args {
    /// Area, used to name the output.
    #[arg(long)]
    pub area: String,
    /// sources.json listing the elevation sources, lowest priority first.
    #[arg(long)]
    pub sources: Option<PathBuf>,
    #[arg(long, default_value_t = 5)]
    pub minzoom: u8,
    #[arg(long, default_value_t = 13)]
    pub maxzoom: u8,
    /// Elevation packing. terrarium is a metre per step at round-digits 8.
    #[arg(long, default_value = "terrarium")]
    pub encoding: String,
    /// Quantisation exponent at the maximum zoom.
    #[arg(long, default_value_t = 8)]
    pub round_digits: u32,
    /// Cap on the per-zoom quantisation ramp.
    #[arg(long, default_value_t = 15)]
    pub max_round_digits: u32,
    /// Metres over which a higher-priority source fades in at its coverage boundary.
    #[arg(long, default_value_t = 1000.0)]
    pub blur: f64,
    #[arg(long, default_value_t = 512)]
    pub tile_size: u32,
    /// west,south,east,north. Defaults to the area's basemap bounds.
    #[arg(long)]
    pub bounds: Option<String>,
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Rebuild even if this step is recorded as already built for the area.
    #[arg(long)]
    pub force: bool,
}

/// The option values this run is defined by, for the build record.
///
/// Only the ones that change the output: `--output` and `--force` decide where it goes and
/// whether to run, not what gets written.
fn terrain_options(args: &Args) -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut values = std::collections::BTreeMap::new();
    values.insert("minzoom".into(), args.minzoom.into());
    values.insert("maxzoom".into(), args.maxzoom.into());
    values.insert("encoding".into(), args.encoding.clone().into());
    values.insert("round_digits".into(), args.round_digits.into());
    values.insert("max_round_digits".into(), args.max_round_digits.into());
    values.insert("blur".into(), args.blur.into());
    values.insert("tile_size".into(), args.tile_size.into());
    if let Some(bounds) = &args.bounds {
        values.insert("bounds".into(), bounds.clone().into());
    }
    values
}

fn parse_bounds(raw: &str) -> Result<(f64, f64, f64, f64)> {
    let parts: Vec<f64> = raw.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [w, s, e, n] => Ok((*w, *s, *e, *n)),
        _ => Err(anyhow!("bounds want west,south,east,north")),
    }
}

pub fn run(settings: &Settings, args: Args) -> Result<()> {
    let area_dir = settings.area_dir(&args.area);
    let recorded = terrain_options(&args);
    if !args.force && state::status(&area_dir, &args.area, StepId::TerrainRgb, &recorded).is_fresh() {
        println!("Terrain RGB is already built for {} - pass --force to rebuild", args.area);
        return Ok(());
    }
    let encoding = Encoding::parse(&args.encoding)
        .ok_or_else(|| anyhow!("unknown encoding `{}`", args.encoding))?;
    let sources_path = args.sources.unwrap_or_else(|| settings.sources_json.clone());
    let specs = source::read_specs(&sources_path)?;
    let (mut composite, skipped) = source::CompositeSource::open(&specs)?;
    println!("sources, highest priority first: {:?}", composite.names());
    for note in &skipped {
        println!("  skipped {note}");
    }
    if composite.names().is_empty() {
        return Err(anyhow!("no usable elevation sources in {}", sources_path.display()));
    }

    // the basemap's bounds keep terrain and vector coverage identical
    let bounds = match &args.bounds {
        Some(raw) => parse_bounds(raw)?,
        None => studio_core::catalog::discover(&settings.output_root)?
            .into_iter()
            .find(|a| a.name == args.area)
            .and_then(|a| {
                a.artifacts
                    .iter()
                    .find(|x| x.kind == studio_core::catalog::ArtifactKind::Basemap)?
                    .bounds
                    .clone()
            })
            .as_deref()
            .map(parse_bounds)
            .transpose()?
            .ok_or_else(|| anyhow!("no --bounds given and no basemap to take them from"))?,
    };

    let opts = render::TerrainOptions {
        encoding,
        minzoom: args.minzoom,
        maxzoom: args.maxzoom,
        tile_size: args.tile_size,
        round_digits: args.round_digits,
        max_round_digits: args.max_round_digits,
        blur_m: args.blur,
    };
    let output = args
        .output
        .unwrap_or_else(|| settings.area_dir(&args.area).join(format!("{}_terrain.mbtiles", args.area)));

    println!(
        "bounds {:.4},{:.4},{:.4},{:.4}  z{}-{}  -> {}",
        bounds.0, bounds.1, bounds.2, bounds.3, opts.minzoom, opts.maxzoom, output.display()
    );

    let conn = render::create_archive(&output, &format!("{}_terrain", args.area), &opts, bounds)?;
    let mut stmt = conn.prepare("INSERT INTO tiles VALUES (?, ?, ?, ?)")?;
    let started = std::time::Instant::now();
    let mut written = 0u64;

    for zoom in opts.minzoom..=opts.maxzoom {
        let (x0, y0, x1, y1) = render::tile_range(zoom, bounds);
        let total = ((x1 - x0 + 1) as u64) * ((y1 - y0 + 1) as u64);
        let step = render::step_for(opts.encoding, zoom, opts.maxzoom, opts.round_digits, opts.max_round_digits);
        let mut done = 0u64;
        for x in x0..=x1 {
            for y in y0..=y1 {
                done += 1;
                if let Some(rgb) = render::render_tile(&mut composite, zoom, x, y, &opts) {
                    let webp = render::to_webp(&rgb, opts.tile_size)?;
                    // mbtiles rows count up from the south
                    stmt.execute((zoom, x, (1u32 << zoom) - 1 - y, &webp))?;
                    written += 1;
                }
                if done % 32 == 0 || done == total {
                    print!("\r  z{zoom} ({step} m steps) {done}/{total}   ");
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
            }
        }
        println!();
    }
    drop(stmt);
    conn.execute_batch("CREATE UNIQUE INDEX tile_index ON tiles (zoom_level, tile_column, tile_row)")?;

    state::mark_done(
        &area_dir,
        StepId::TerrainRgb,
        Some(super::planetiler::human_elapsed(started.elapsed())),
        &recorded,
    )?;

    let size = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    println!(
        "wrote {written} tiles, {} in {:.1}s",
        super::mb(size),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
