//! Rendering terrain-RGB tiles.
//!
//! Ports the encoding half of `scripts/build_terrain_rgb.py`. The vertical quantisation ramp is
//! the part that matters for size: elevation is snapped to a step that grows as zoom drops, so
//! the low-detail zooms spend far fewer distinct byte values and compress much harder.
//!
//! For terrarium the step is `(1/256) * 2^round_digits`, so `round_digits = 8` is exactly one
//! metre - and at whole metres the blue channel, which carries the sub-metre fraction, becomes a
//! constant zero. That cliff is where terrarium's size advantage over mapbox comes from; at
//! matched fractional steps mapbox is the smaller of the two.

use crate::elevation::Encoding;
use crate::terrain::source::CompositeSource;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Base value and quantisation interval per encoding, matching the Python `ENCODINGS` table.
pub fn interval(encoding: Encoding) -> f64 {
    match encoding {
        // mapbox's interval is fixed by the format at 0.1 m and is not a free parameter
        Encoding::Mapbox => 0.1,
        Encoding::Terrarium => 1.0 / 256.0,
    }
}

/// Quantisation exponent for a zoom.
///
/// Detail is only needed where it can be seen, so each step down from the maximum zoom adds one
/// to the exponent - doubling the vertical step - until `max_round_digits` caps it.
pub fn round_digits_for(zoom: u8, maxzoom: u8, round_digits: u32, max_round_digits: u32) -> u32 {
    let ramp = round_digits + (maxzoom.saturating_sub(zoom)) as u32;
    ramp.min(max_round_digits.max(round_digits))
}

/// Vertical step in metres at a zoom.
pub fn step_for(encoding: Encoding, zoom: u8, maxzoom: u8, round_digits: u32, max_round_digits: u32) -> f64 {
    let digits = round_digits_for(zoom, maxzoom, round_digits, max_round_digits);
    interval(encoding) * 2f64.powi(digits as i32)
}

/// Pack an elevation into RGB, snapped to `step`.
pub fn encode(encoding: Encoding, elevation: f32, step: f64) -> [u8; 3] {
    let snapped = if step > 0.0 {
        ((elevation as f64 / step).round() * step) as f32
    } else {
        elevation
    };
    match encoding {
        Encoding::Terrarium => {
            let v = (snapped as f64 + 32768.0).clamp(0.0, 65535.999);
            let whole = v.floor();
            let r = (whole / 256.0).floor() as u8;
            let g = (whole % 256.0) as u8;
            let b = ((v - whole) * 256.0).round().clamp(0.0, 255.0) as u8;
            [r, g, b]
        }
        Encoding::Mapbox => {
            let v = ((snapped as f64 + 10000.0) / 0.1).round().clamp(0.0, 16_777_215.0) as u32;
            [(v >> 16) as u8, (v >> 8) as u8, v as u8]
        }
    }
}

/// Longitude and latitude of a pixel centre in an XYZ tile.
pub fn pixel_lonlat(z: u8, x: u32, y: u32, px: u32, py: u32, size: u32) -> (f64, f64) {
    let n = (1u64 << z) as f64;
    let world_x = (x as f64 * size as f64 + px as f64 + 0.5) / (size as f64 * n);
    let world_y = (y as f64 * size as f64 + py as f64 + 0.5) / (size as f64 * n);
    let lon = world_x * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * world_y)).sinh().atan().to_degrees();
    (lon, lat)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainOptions {
    pub encoding: Encoding,
    pub minzoom: u8,
    pub maxzoom: u8,
    pub tile_size: u32,
    /// Quantisation exponent at the maximum zoom. 8 gives whole metres for terrarium.
    pub round_digits: u32,
    /// Cap on the ramp. Note the Python original silently clamps this up to `round_digits` when
    /// it is smaller, which with its default of 0 disables the ramp entirely.
    pub max_round_digits: u32,
    /// Distance over which a higher-priority source fades in at its coverage boundary, in
    /// metres. Matches the generator's `--blur`, whose default is also 1000.
    pub blur_m: f64,
    /// Elevation written where no source covers the pixel.
    ///
    /// Only reachable inside a tile that is covered somewhere - a tile no source touches at all
    /// is skipped rather than written flat. `build_terrain_rgb.py` used -10 so that uncovered
    /// pixels read as sea rather than as ground; 0 is kept as the default here because it is
    /// what every archive in this repository was built with.
    pub nodata_elevation: f64,
}

impl Default for TerrainOptions {
    fn default() -> Self {
        Self {
            encoding: Encoding::Terrarium,
            minzoom: 5,
            maxzoom: 13,
            tile_size: 512,
            nodata_elevation: 0.0,
            round_digits: 8,
            max_round_digits: 15,
            blur_m: 1000.0,
        }
    }
}

/// Ground resolution of a tile's pixels, in metres.
///
/// Used to pick which overview of a projected raster source to read: sampling a z8 tile out of
/// 5 m data would decode thousands of full-resolution raster tiles for one output tile.
pub fn ground_resolution(z: u8, y: u32, size: u32) -> f64 {
    let n = (1u64 << z) as f64;
    let world_y = (y as f64 + 0.5) / n;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * world_y)).sinh().atan();
    40_075_016.686 * lat.cos() / (n * size as f64)
}

/// Render one tile. Returns `None` when no pixel had coverage, so empty tiles are skipped
/// rather than written as a wall of sea level.
pub fn render_tile(source: &mut CompositeSource, z: u8, x: u32, y: u32, opts: &TerrainOptions) -> Option<Vec<u8>> {
    let size = opts.tile_size;
    let step = step_for(opts.encoding, z, opts.maxzoom, opts.round_digits, opts.max_round_digits);
    let target = ground_resolution(z, y, size);
    let mut rgb = vec![0u8; (size * size * 3) as usize];
    let mut covered = false;

    for py in 0..size {
        // longitude depends only on the column and latitude only on the row, so the row's
        // latitude is computed once rather than per pixel
        let (_, lat) = pixel_lonlat(z, x, y, 0, py, size);
        for px in 0..size {
            let (lon, _) = pixel_lonlat(z, x, y, px, py, size);
            let elevation = match source.sample_blended(lon, lat, target, opts.blur_m) {
                Some(e) => {
                    covered = true;
                    e
                }
                None => opts.nodata_elevation as f32,
            };
            let packed = encode(opts.encoding, elevation, step);
            let at = ((py * size + px) * 3) as usize;
            rgb[at..at + 3].copy_from_slice(&packed);
        }
    }
    covered.then_some(rgb)
}

/// Encode an RGB buffer as lossless WebP.
pub fn to_webp(rgb: &[u8], size: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out)
        .encode(rgb, size, size, image::ExtendedColorType::Rgb8)
        .context("encoding webp")?;
    Ok(out)
}

/// Encode an RGB buffer as PNG.
///
/// Bigger than lossless WebP for this data, but some tools still will not read WebP - which is
/// why `build_terrain_rgb.py` had `-f png` and this does too.
pub fn to_png(rgb: &[u8], size: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    // the encoder trait has to be in scope for `write_image`; webp's inherent method does not
    // need it, which is why only this one imports it
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(rgb, size, size, image::ExtendedColorType::Rgb8)
        .context("encoding png")?;
    Ok(out)
}

/// Lon/lat box of a web-mercator tile, for deciding whether a shape touches it.
pub fn tile_bounds(z: u8, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = (1u64 << z) as f64;
    let lon = |x: f64| x / n * 360.0 - 180.0;
    let lat = |y: f64| {
        let t = std::f64::consts::PI * (1.0 - 2.0 * y / n);
        t.sinh().atan().to_degrees()
    };
    (lon(x as f64), lat(y as f64 + 1.0), lon(x as f64 + 1.0), lat(y as f64))
}

/// Tile range covering a lon/lat bounding box at a zoom.
pub fn tile_range(z: u8, bounds: (f64, f64, f64, f64)) -> (u32, u32, u32, u32) {
    let n = (1u64 << z) as f64;
    let to_x = |lon: f64| (((lon + 180.0) / 360.0 * n).floor().max(0.0) as u32).min((n - 1.0) as u32);
    let to_y = |lat: f64| {
        let rad = lat.to_radians();
        let v = (1.0 - (rad.tan() + 1.0 / rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
        (v.floor().max(0.0) as u32).min((n - 1.0) as u32)
    };
    // latitude runs the other way from tile rows
    (to_x(bounds.0), to_y(bounds.3), to_x(bounds.2), to_y(bounds.1))
}

/// Open an mbtiles for writing and lay out the schema and metadata.
pub fn create_archive(path: &Path, name: &str, opts: &TerrainOptions, bounds: (f64, f64, f64, f64)) -> Result<Connection> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=OFF;
         PRAGMA synchronous=OFF;
         CREATE TABLE metadata (name text, value text);
         CREATE TABLE tiles (zoom_level integer, tile_column integer,
           tile_row integer, tile_data blob);",
    )?;
    let encoding = match opts.encoding {
        Encoding::Terrarium => "terrarium",
        Encoding::Mapbox => "mapbox",
    };
    for (key, value) in [
        ("name", name.to_string()),
        ("format", "webp".into()),
        ("type", "baselayer".into()),
        ("version", "1".into()),
        ("description", format!("{encoding} terrain rgb")),
        // MapLibre needs this exact key to decode elevation from the tiles
        ("encoding", encoding.into()),
        ("minzoom", opts.minzoom.to_string()),
        ("maxzoom", opts.maxzoom.to_string()),
        ("bounds", format!("{},{},{},{}", bounds.0, bounds.1, bounds.2, bounds.3)),
    ] {
        conn.execute("INSERT INTO metadata VALUES (?, ?)", (key, value))?;
    }
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrarium_step_of_one_metre_at_eight_digits() {
        let s = step_for(Encoding::Terrarium, 13, 13, 8, 15);
        assert!((s - 1.0).abs() < 1e-12, "got {s}");
    }

    /// Each zoom below the maximum doubles the step, until the cap.
    #[test]
    fn the_ramp_doubles_per_zoom_and_then_caps() {
        let step = |z| step_for(Encoding::Terrarium, z, 13, 8, 11);
        assert_eq!(step(13), 1.0);
        assert_eq!(step(12), 2.0);
        assert_eq!(step(11), 4.0);
        assert_eq!(step(10), 8.0);
        assert_eq!(step(9), 8.0, "capped at max_round_digits");
    }

    /// The Python original raises `max_round_digits` to `round_digits` when it is smaller, so
    /// its default of 0 disables the ramp entirely. Matching that keeps ports comparable.
    #[test]
    fn cap_below_the_base_disables_the_ramp() {
        for z in 5..=13 {
            assert_eq!(step_for(Encoding::Terrarium, z, 13, 8, 0), 1.0);
        }
    }

    #[test]
    fn terrarium_encodes_the_documented_values() {
        assert_eq!(encode(Encoding::Terrarium, 0.0, 0.0), [128, 0, 0]);
        assert_eq!(encode(Encoding::Terrarium, 100.0, 0.0), [128, 100, 0]);
        assert_eq!(encode(Encoding::Terrarium, 500.0, 0.0), [129, 244, 0]);
    }

    /// Where terrarium's size advantage actually comes from: at whole-metre steps the blue
    /// channel, which carries the fraction, is constant zero across the whole tile.
    #[test]
    fn whole_metre_quantisation_zeroes_the_blue_channel() {
        for elevation in [0.4f32, 123.7, 2401.2, -5.9] {
            let [_, _, b] = encode(Encoding::Terrarium, elevation, 1.0);
            assert_eq!(b, 0, "blue must vanish at 1 m steps for {elevation}");
        }
        // and it does not vanish at a fractional step
        assert_ne!(encode(Encoding::Terrarium, 123.5, 1.0 / 256.0)[2], 0);
    }

    #[test]
    fn round_trips_through_the_decoder() {
        for elevation in [-400.0f32, 0.0, 137.0, 2000.0, 4808.0] {
            for enc in [Encoding::Terrarium, Encoding::Mapbox] {
                let [r, g, b] = encode(enc, elevation, 1.0);
                let back = enc.decode(r, g, b);
                assert!((back - elevation).abs() <= 1.0, "{enc:?} {elevation} -> {back}");
            }
        }
    }

    #[test]
    fn pixel_centres_land_inside_their_tile() {
        // z0 has one tile; its centre pixel is near null island
        let (lon, lat) = pixel_lonlat(0, 0, 0, 256, 256, 512);
        assert!(lon.abs() < 1.0 && lat.abs() < 1.0, "got {lon},{lat}");
        // the north-west pixel is near the projection's corner
        let (lon, lat) = pixel_lonlat(0, 0, 0, 0, 0, 512);
        assert!(lon < -179.0 && lat > 85.0, "got {lon},{lat}");
    }

    #[test]
    fn tile_range_covers_the_alps() {
        let (x0, y0, x1, y1) = tile_range(8, (3.68, 44.11, 7.19, 46.52));
        assert!(x0 <= x1 && y0 <= y1, "range must be ordered: {x0},{y0}..{x1},{y1}");
        // sanity: the box is a handful of tiles at z8, not the whole world
        assert!(x1 - x0 < 10 && y1 - y0 < 10);
    }

    fn hgt_only(dir: &std::path::Path) -> CompositeSource {
        let specs = vec![crate::terrain::source::SourceSpec {
            name: "hgt".into(), kind: "valhalla".into(),
            path: dir.to_path_buf(), clamp_min: None,
        }];
        CompositeSource::open(&specs).unwrap().0
    }

    #[test]
    fn empty_coverage_yields_no_tile() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = hgt_only(dir.path());
        assert!(render_tile(&mut src, 8, 131, 91, &TerrainOptions::default()).is_none());
    }

    /// Ground resolution has to shrink with zoom and with latitude, or the wrong raster overview
    /// gets read and low zooms crawl.
    #[test]
    fn ground_resolution_follows_zoom_and_latitude() {
        let equator_z8 = ground_resolution(8, 128, 512);
        let equator_z12 = ground_resolution(12, 2048, 512);
        assert!(equator_z12 < equator_z8 / 8.0, "{equator_z12} vs {equator_z8}");
        // around 45 degrees a z13 512-pixel tile is a few metres per pixel
        let alps = ground_resolution(13, 2963, 512);
        assert!((5.0..12.0).contains(&alps), "got {alps}");
    }

    #[test]
    fn renders_and_encodes_a_tile() {
        let dir = tempfile::tempdir().unwrap();
        let size = 101usize;
        let mut grid = Vec::new();
        for row in 0..size {
            for _ in 0..size {
                grid.extend_from_slice(&((row * 10) as i16).to_be_bytes());
            }
        }
        std::fs::write(dir.path().join("N44E006.hgt"), grid).unwrap();
        let mut src = hgt_only(dir.path());

        let opts = TerrainOptions { tile_size: 64, maxzoom: 8, ..Default::default() };
        let (x0, y0, _, _) = tile_range(8, (6.1, 44.1, 6.9, 44.9));
        let rgb = render_tile(&mut src, 8, x0, y0, &opts).expect("covered");
        assert_eq!(rgb.len(), 64 * 64 * 3);

        let webp = to_webp(&rgb, 64).unwrap();
        assert_eq!(&webp[..4], b"RIFF");
        // and it decodes back to the same pixels
        let decoded = image::load_from_memory(&webp).unwrap().to_rgb8();
        assert_eq!(decoded.as_raw().len(), rgb.len());
    }

    #[test]
    fn archive_metadata_matches_what_the_viewer_needs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t_terrain.mbtiles");
        let opts = TerrainOptions::default();
        let conn = create_archive(&path, "t_terrain", &opts, (3.0, 44.0, 7.0, 46.0)).unwrap();
        drop(conn);

        let art = crate::catalog::probe(&path, "t");
        assert_eq!(art.kind, crate::catalog::ArtifactKind::TerrainRgb);
        assert_eq!(art.encoding.as_deref(), Some("terrarium"));
        assert_eq!(art.maxzoom, Some(13));
    }
}
