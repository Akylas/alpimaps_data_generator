//! `cairn terrain` - terrain-RGB tiles, mirroring build_terrain_rgb.py's flags.

use anyhow::{anyhow, Result};
use clap::Args as ClapArgs;
use std::path::PathBuf;
use cairn_core::elevation::Encoding;
use cairn_core::settings::Settings;
use cairn_core::steps::{state, StepId};
use cairn_core::terrain::{render, source};

#[derive(ClapArgs)]
pub struct Args {
    /// Area, used to name the output.
    #[arg(long)]
    pub area: String,
    /// sources.json listing the elevation sources, lowest priority first.
    #[arg(long)]
    pub sources: Option<PathBuf>,
    /// Where the .hgt elevation tiles are, when sources.json does not name a directory.
    #[arg(long)]
    pub elevation_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 5)]
    pub minzoom: u8,
    #[arg(long, default_value_t = 12)]
    pub maxzoom: u8,
    /// Elevation packing. terrarium is a metre per step at round-digits 8.
    #[arg(long, default_value = "mapbox")]
    pub encoding: String,
    /// Quantisation exponent at the maximum zoom.
    #[arg(long, default_value_t = 0)]
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
    /// Osmosis .poly limiting which tiles are written, as in build_terrain_rgb.py.
    #[arg(long)]
    pub poly_shape: Option<PathBuf>,
    /// Ring of extra tiles around the shape. 3D renderers backfill a tile's 1px border from its
    /// neighbours, so a ring removes the seam at the edge of the covered area.
    #[arg(long, default_value_t = 1)]
    pub tile_buffer: u32,
    /// Tile encoding on disk.
    #[arg(long, short = 'f', default_value = "webp", value_parser = ["webp", "png"])]
    pub format: String,
    /// Worker threads. Defaults to the number of cores.
    #[arg(long, short = 'j')]
    pub workers: Option<usize>,
    /// Elevation written where no source covers a pixel. build_terrain_rgb.py used -10.
    #[arg(long, default_value_t = 0.0)]
    pub nodata_elevation: f64,
    /// Do not fetch missing .hgt tiles; render only what is already on disk.
    #[arg(long)]
    pub no_elevation_download: bool,
    /// Stop if the output is already there, instead of replacing it.
    #[arg(long)]
    pub skip_existing: bool,
    /// Print what would be built, and stop.
    #[arg(long)]
    pub dry_run: bool,
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
    values.insert("format".into(), args.format.clone().into());
    values.insert("tile_buffer".into(), args.tile_buffer.into());
    values.insert("nodata_elevation".into(), args.nodata_elevation.into());
    if let Some(poly) = &args.poly_shape {
        values.insert("poly_shape".into(), poly.display().to_string().into());
    }
    if let Some(bounds) = &args.bounds {
        values.insert("bounds".into(), bounds.clone().into());
    }
    values
}

/// Whether a tile is inside the shape, or within `buffer` tiles of it.
///
/// The buffer is measured in tiles at this zoom, so the ring is the same width everywhere on the
/// pyramid rather than a fixed distance that means different things at z5 and z13.
fn touches(shape: &cairn_core::poly::Polygon, z: u8, x: u32, y: u32, buffer: u32) -> bool {
    let (w, s, e, n) = render::tile_bounds(z, x, y);
    if buffer == 0 {
        return shape.intersects_rect(w, s, e, n);
    }
    let dx = (e - w) * buffer as f64;
    let dy = (n - s) * buffer as f64;
    shape.intersects_rect(w - dx, s - dy, e + dx, n + dy)
}

fn parse_bounds(raw: &str) -> Result<(f64, f64, f64, f64)> {
    let parts: Vec<f64> = raw.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [w, s, e, n] => Ok((*w, *s, *e, *n)),
        _ => Err(anyhow!("bounds want west,south,east,north")),
    }
}

pub async fn run(settings: &Settings, args: Args) -> Result<()> {
    let area_dir = settings.area_dir(&args.area);
    let recorded = terrain_options(&args);
    let encoding = Encoding::parse(&args.encoding)
        .ok_or_else(|| anyhow!("unknown encoding `{}`", args.encoding))?;
    let suffix = "terrain";
    let sources_path = args.sources.clone().unwrap_or_else(|| settings.sources_json.clone());
    let hgt_dir = args
        .elevation_dir
        .clone()
        .unwrap_or_else(|| settings.elevation_tiles_dir.clone());
    // sources.json is the pipeline's own list, in priority order; without one, the bare .hgt
    // directory is a usable source on its own, which is what a fresh install has
    let specs = match source::read_specs(&sources_path) {
        Ok(specs) => specs,
        Err(e) if hgt_dir.is_dir() => {
            println!("no {} ({e}); reading {}", sources_path.display(), hgt_dir.display());
            vec![source::SourceSpec {
                name: "elevation_tiles".into(),
                kind: "valhalla".into(),
                path: hgt_dir.clone(),
                clamp_min: Some(-10.0),
                download: None,
            }]
        }
        Err(e) => return Err(e),
    };
    let (mut composite, skipped) = source::CompositeSource::open(&specs)?;
    println!("sources, highest priority first: {:?}", composite.names());
    for note in &skipped {
        println!("  skipped {note}");
    }
    if composite.names().is_empty() {
        return Err(anyhow!("no usable elevation sources in {}", sources_path.display()));
    }

    // the basemap's bounds keep terrain and vector coverage identical
    // the shape, when given, is both the clip and the default extent
    let shape = args
        .poly_shape
        .as_ref()
        .map(|p| cairn_core::poly::Polygon::parse(p))
        .transpose()?;

    let bounds = match (&args.bounds, &shape) {
        (Some(raw), _) => parse_bounds(raw)?,
        (None, Some(shape)) => shape.bounds(),
        (None, None) => cairn_core::catalog::discover(&settings.output_root)?
            .into_iter()
            .find(|a| a.name == args.area)
            .and_then(|a| {
                a.artifacts
                    .iter()
                    .find(|x| x.kind == cairn_core::catalog::ArtifactKind::Basemap)?
                    .bounds
                    .clone()
            })
            .as_deref()
            .map(parse_bounds)
            .transpose()?
            .ok_or_else(|| {
                anyhow!("no --bounds or --poly-shape given, and no basemap to take them from")
            })?,
    };

    let opts = render::TerrainOptions {
        encoding,
        minzoom: args.minzoom,
        maxzoom: args.maxzoom,
        tile_size: args.tile_size,
        round_digits: args.round_digits,
        max_round_digits: args.max_round_digits,
        blur_m: args.blur,
        nodata_elevation: args.nodata_elevation,
    };
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| area_dir.join(format!("{}_{suffix}.mbtiles", args.area)));
    if args.skip_existing && output.is_file() {
        println!("{} is already there", output.display());
        return Ok(());
    }
    if let Some(workers) = args.workers {
        // best effort: a second build in the same process would keep the first pool
        let _ = rayon::ThreadPoolBuilder::new().num_threads(workers).build_global();
    }
    if args.dry_run {
        let mut tiles = 0u64;
        for zoom in opts.minzoom..=opts.maxzoom {
            let (x0, y0, x1, y1) = render::tile_range(zoom, bounds);
            tiles += ((x1 - x0 + 1) as u64) * ((y1 - y0 + 1) as u64);
        }
        println!(
            "z{}-{} over {:.4},{:.4},{:.4},{:.4}: up to {tiles} tiles -> {}",
            opts.minzoom, opts.maxzoom, bounds.0, bounds.1, bounds.2, bounds.3, output.display()
        );
        return Ok(());
    }

    // a missing .hgt is silent - the renderer writes nothing there and the archive comes out
    // with a hole - so the tiles this render needs are fetched first
    if !args.no_elevation_download {
        let (got, total) =
            cairn_core::steps::elevation::ensure(&hgt_dir, bounds, |done, total| {
                print!("\r  elevation {done}/{total}   ");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            })
            .await?;
        if total > 0 {
            println!("\r  elevation: {got} downloaded of {total} covering tiles");
        }
    }

    println!(
        "bounds {:.4},{:.4},{:.4},{:.4}  z{}-{}  -> {}",
        bounds.0, bounds.1, bounds.2, bounds.3, opts.minzoom, opts.maxzoom, output.display()
    );

    let conn = render::create_archive(&output, &format!("{}_{suffix}", args.area), &opts, bounds)?;
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
                if let Some(shape) = &shape {
                    if !touches(shape, zoom, x, y, args.tile_buffer) {
                        continue;
                    }
                }
                if let Some(rgb) = render::render_tile(&mut composite, zoom, x, y, &opts) {
                    let webp = if args.format == "png" {
                        render::to_png(&rgb, opts.tile_size)?
                    } else {
                        render::to_webp(&rgb, opts.tile_size)?
                    };
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

#[cfg(test)]
mod readme_defaults_tests {
    use super::Args;
    use clap::Parser;

    #[derive(Parser)]
    struct Wrapper {
        #[command(flatten)]
        args: Args,
    }

    /// An untouched `cairn terrain` must reproduce the README's build_terrain_rgb command.
    ///
    /// These drifted once already: cairn defaulted to maxzoom 13, terrarium, round-digits 8 and no
    /// tile buffer, where the README asks for 12, mapbox, 0 and 1. Nothing caught it, and the two
    /// silently produced different archives.
    #[test]
    fn terrain_defaults_match_the_readme_command() {
        let readme = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md"),
        )
        .expect("repo README should be two levels above cairn/cli");
        let lines: Vec<&str> = readme.lines().collect();
        let start = lines
            .iter()
            // the README also mentions the script in prose; only the invocation starts with python
            .position(|l| l.trim_start().starts_with("python") && l.contains("build_terrain_rgb.py"))
            .expect("a README build_terrain_rgb command");
        let mut cmd = String::new();
        for l in &lines[start..] {
            cmd.push_str(l.trim_end_matches('\\'));
            cmd.push(' ');
            if !l.trim_end().ends_with('\\') {
                break;
            }
        }

        let args = Wrapper::parse_from(["cairn-terrain", "--area", "test"]).args;
        for (flag, actual) in [
            ("--minzoom", args.minzoom.to_string()),
            ("--maxzoom", args.maxzoom.to_string()),
            ("--round-digits", args.round_digits.to_string()),
            ("--encoding", args.encoding.clone()),
            ("--blur", format!("{:.0}", args.blur)),
            ("--tile-buffer", args.tile_buffer.to_string()),
        ] {
            let want = cmd
                .split_whitespace()
                .skip_while(|t| *t != flag)
                .nth(1)
                .unwrap_or_else(|| panic!("README terrain command has no {flag}"));
            assert_eq!(
                actual, want,
                "cairn terrain defaults {flag}={actual}, README asks for {want}"
            );
        }
    }
}
