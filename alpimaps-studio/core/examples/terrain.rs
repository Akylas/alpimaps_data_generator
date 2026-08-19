//! Render terrain-RGB tiles from `.hgt` sources and compare against an existing archive.
//!
//!   cargo run --release -p studio-core --example terrain -- \
//!       ../elevation_tiles ../alpimaps_mbtiles/rhone-alpes/rhone-alpes_terrain.mbtiles [z]

use studio_core::elevation::{Encoding, TerrainSampler};
use studio_core::terrain::{render, source};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let sources_json = args.next().expect("usage: terrain <sources.json> <reference.mbtiles> [zoom]");
    let reference = args.next().expect("reference archive required");
    let zoom: u8 = args.next().and_then(|z| z.parse().ok()).unwrap_or(10);

    let specs = source::read_specs(std::path::Path::new(&sources_json))?;
    let (mut source, skipped) = source::CompositeSource::open(&specs)?;
    println!("sources in priority order: {:?}", source.names());
    for note in &skipped {
        println!("  skipped {note}");
    }

    let blur: f64 = std::env::var("BLUR").ok().and_then(|v| v.parse().ok()).unwrap_or(1000.0);
    let opts = render::TerrainOptions { maxzoom: 13, blur_m: blur, ..Default::default() };
    println!("source blend: {} m", opts.blur_m);
    let step = render::step_for(opts.encoding, zoom, opts.maxzoom, opts.round_digits, opts.max_round_digits);
    println!("z{zoom}: vertical step {step} m, tile size {}", opts.tile_size);

    // a handful of tiles across the middle of the area
    let bounds = (5.0, 45.0, 6.5, 45.8);
    let (x0, y0, x1, y1) = render::tile_range(zoom, bounds);
    let mut reference_sampler = TerrainSampler::open(std::path::Path::new(&reference))?;

    let (mut compared, mut sum_abs, mut worst) = (0usize, 0.0f64, 0.0f32);
    let mut within_5m = 0usize;
    let started = std::time::Instant::now();
    let mut rendered = 0usize;
    let mut bytes = 0usize;

    for x in x0..=x1.min(x0 + 2) {
        for y in y0..=y1.min(y0 + 2) {
            let Some(rgb) = render::render_tile(&mut source, zoom, x, y, &opts) else { continue };
            rendered += 1;
            bytes += render::to_webp(&rgb, opts.tile_size)?.len();

            // sample both at the same points and compare elevations
            let size = opts.tile_size;
            for py in (0..size).step_by(37) {
                for px in (0..size).step_by(37) {
                    let (lon, lat) = render::pixel_lonlat(zoom, x, y, px, py, size);
                    let at = ((py * size + px) * 3) as usize;
                    let mine = Encoding::Terrarium.decode(rgb[at], rgb[at + 1], rgb[at + 2]);
                    let Some(theirs) = reference_sampler.sample(lon, lat, zoom) else { continue };
                    let diff = (mine - theirs).abs();
                    compared += 1;
                    sum_abs += diff as f64;
                    worst = worst.max(diff);
                    if diff <= 5.0 {
                        within_5m += 1;
                    }
                }
            }
        }
    }

    println!(
        "rendered {rendered} tiles in {:.1}s, {:.1} KB of webp",
        started.elapsed().as_secs_f64(),
        bytes as f64 / 1024.0
    );
    if compared > 0 {
        println!(
            "\ncompared {compared} points against the reference archive:\n  \
             mean |delta| {:.1} m, worst {:.0} m, within 5 m: {:.0}%",
            sum_abs / compared as f64,
            worst,
            100.0 * within_5m as f64 / compared as f64
        );
        let step = render::step_for(opts.encoding, zoom, opts.maxzoom, opts.round_digits, opts.max_round_digits);
        println!(
            "\nVertical quantisation at this zoom is {step} m, so a delta of one step is\n\
             agreement, not disagreement. Both sides read the same sources.json."
        );
    }
    Ok(())
}
