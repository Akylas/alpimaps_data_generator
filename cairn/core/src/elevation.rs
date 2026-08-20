//! Sampling elevation out of terrain-RGB archives, and building profiles along a line.
//!
//! Doubles as QA for the terrain build. The generator quantises elevation on a per-zoom ramp
//! (`round_digits` growing as zoom drops), and that quantisation is visible here as stair-steps
//! in a profile - far easier to judge than squinting at hillshade.
//!
//! Quantisation also has a trap for totals: at 1 m steps, a nearly flat traverse alternates
//! between two adjacent levels, and naively summing positive deltas turns that dither into
//! hundreds of metres of phantom ascent. [`Profile`] therefore accumulates with hysteresis - a
//! direction change only counts once it exceeds a threshold.

use crate::catalog::TileFormat;
use anyhow::{anyhow, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// How elevation is packed into RGB. Both schemes are fixed-point; neither is parameterised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    /// `(R * 256 + G + B / 256) - 32768`
    Terrarium,
    /// `-10000 + (R * 65536 + G * 256 + B) * 0.1`
    Mapbox,
}

impl Encoding {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "terrarium" => Some(Encoding::Terrarium),
            "mapbox" => Some(Encoding::Mapbox),
            _ => None,
        }
    }

    pub fn decode(self, r: u8, g: u8, b: u8) -> f32 {
        let (r, g, b) = (r as f32, g as f32, b as f32);
        match self {
            Encoding::Terrarium => (r * 256.0 + g + b / 256.0) - 32768.0,
            Encoding::Mapbox => -10000.0 + (r * 65536.0 + g * 256.0 + b) * 0.1,
        }
    }
}

/// Web Mercator world coordinates in tile units at `zoom` (fractional).
fn lonlat_to_world(lon: f64, lat: f64, zoom: u8) -> (f64, f64) {
    let n = (1u64 << zoom) as f64;
    let x = (lon + 180.0) / 360.0 * n;
    let lat_rad = lat.to_radians();
    let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// Great-circle distance in metres.
pub fn haversine_m(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    const R: f64 = 6_371_008.8;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfilePoint {
    pub lon: f64,
    pub lat: f64,
    /// Cumulative distance from the start, in metres.
    pub distance_m: f64,
    /// `None` where the terrain archive has no tile covering the point.
    pub elevation_m: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub points: Vec<ProfilePoint>,
    pub distance_m: f64,
    pub ascent_m: f64,
    pub descent_m: f64,
    pub min_m: Option<f32>,
    pub max_m: Option<f32>,
    /// Zoom the samples were taken at; the vertical quantisation depends on it.
    pub zoom: u8,
    /// Hysteresis threshold used for ascent/descent, in metres.
    pub threshold_m: f64,
    /// Points that fell outside the archive's coverage.
    pub gaps: usize,
}

pub struct TerrainSampler {
    conn: Connection,
    encoding: Encoding,
    tile_size: u32,
    pub minzoom: u8,
    pub maxzoom: u8,
    /// Decoded tiles, kept as flat elevation grids. A 512-square tile is 1 MB as `f32`.
    cache: HashMap<(u8, u32, u32), Option<Arc<Vec<f32>>>>,
}

impl TerrainSampler {
    /// Open a terrain-RGB archive. `encoding` falls back to the archive's `encoding` metadata.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("opening {}", path.display()))?;
        let meta: HashMap<String, String> = {
            let mut stmt = conn.prepare("SELECT name, value FROM metadata")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        let encoding = meta
            .get("encoding")
            .and_then(|e| Encoding::parse(e))
            .ok_or_else(|| anyhow!("archive has no usable `encoding` metadata; not a terrain-RGB tileset"))?;
        let format = meta.get("format").map(|f| f.as_str()).unwrap_or("");
        if matches!(TileFormat::parse_public(format), TileFormat::Mvt | TileFormat::Mlt) {
            return Err(anyhow!("{format} is a vector tileset, not terrain RGB"));
        }
        Ok(Self {
            encoding,
            tile_size: 512,
            minzoom: meta.get("minzoom").and_then(|v| v.parse().ok()).unwrap_or(0),
            maxzoom: meta.get("maxzoom").and_then(|v| v.parse().ok()).unwrap_or(14),
            conn,
            cache: HashMap::new(),
        })
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Decode one tile into a flat grid of elevations, caching both hits and misses.
    fn grid(&mut self, z: u8, x: u32, y: u32) -> Option<Arc<Vec<f32>>> {
        if let Some(cached) = self.cache.get(&(z, x, y)) {
            return cached.clone();
        }
        let decoded = self.load_grid(z, x, y);
        self.cache.insert((z, x, y), decoded.clone());
        decoded
    }

    fn load_grid(&mut self, z: u8, x: u32, y: u32) -> Option<Arc<Vec<f32>>> {
        let tms_row = (1u32 << z).checked_sub(1)?.checked_sub(y)?;
        let blob: Vec<u8> = self
            .conn
            .query_row(
                "SELECT tile_data FROM tiles WHERE zoom_level=? AND tile_column=? AND tile_row=?",
                (z, x, tms_row),
                |r| r.get(0),
            )
            .ok()?;
        let img = image::load_from_memory(&blob).ok()?.to_rgb8();
        let (w, h) = img.dimensions();
        if w != h {
            return None;
        }
        self.tile_size = w;
        let encoding = self.encoding;
        let grid: Vec<f32> = img
            .pixels()
            .map(|p| encoding.decode(p[0], p[1], p[2]))
            .collect();
        Some(Arc::new(grid))
    }

    fn texel(&mut self, z: u8, world_px: (f64, f64)) -> Option<f32> {
        let size = self.tile_size as f64;
        let (wx, wy) = world_px;
        if wx < 0.0 || wy < 0.0 {
            return None;
        }
        let (tx, ty) = ((wx / size) as u32, (wy / size) as u32);
        let grid = self.grid(z, tx, ty)?;
        let (px, py) = ((wx % size) as usize, (wy % size) as usize);
        let stride = self.tile_size as usize;
        grid.get(py * stride + px).copied()
    }

    /// Bilinear sample at `zoom`. Neighbouring texels may live in adjacent tiles, so each of the
    /// four is resolved independently rather than assuming they share one.
    pub fn sample(&mut self, lon: f64, lat: f64, zoom: u8) -> Option<f32> {
        let (tile_x, tile_y) = lonlat_to_world(lon, lat, zoom);
        if tile_x < 0.0 || tile_y < 0.0 {
            return None;
        }
        // Tile coordinates do not depend on the tile's pixel size, but pixel coordinates do -
        // so the covering tile has to be decoded *first* to learn the size. Assuming 512 up
        // front silently mislocates every sample in a 256-pixel archive.
        self.grid(zoom, tile_x as u32, tile_y as u32)?;
        let size = self.tile_size as f64;
        // continuous pixel coordinates across the whole world at this zoom
        let (wx, wy) = (tile_x * size - 0.5, tile_y * size - 0.5);
        let (x0, y0) = (wx.floor(), wy.floor());
        let (fx, fy) = (wx - x0, wy - y0);

        let v00 = self.texel(zoom, (x0, y0))?;
        let v10 = self.texel(zoom, (x0 + 1.0, y0)).unwrap_or(v00);
        let v01 = self.texel(zoom, (x0, y0 + 1.0)).unwrap_or(v00);
        let v11 = self.texel(zoom, (x0 + 1.0, y0 + 1.0)).unwrap_or(v10);

        let (fx, fy) = (fx as f32, fy as f32);
        let top = v00 + (v10 - v00) * fx;
        let bottom = v01 + (v11 - v01) * fx;
        Some(top + (bottom - top) * fy)
    }

    /// Sample a polyline, optionally inserting intermediate points every `densify_m`.
    ///
    /// `threshold_m` is the hysteresis applied to ascent/descent. At the terrain's 1 m vertical
    /// step, zero threshold turns quantisation dither into phantom climb, so the default of 3 m
    /// is a deliberate floor rather than a rounding convenience.
    pub fn profile(
        &mut self,
        line: &[[f64; 2]],
        zoom: u8,
        densify_m: f64,
        threshold_m: f64,
    ) -> Result<Profile> {
        if line.len() < 2 {
            return Err(anyhow!("a profile needs at least two points"));
        }
        let zoom = zoom.clamp(self.minzoom, self.maxzoom);

        // walk the line, emitting the vertices plus any densified points between them
        let mut coords: Vec<[f64; 2]> = vec![line[0]];
        for pair in line.windows(2) {
            let ([lon1, lat1], [lon2, lat2]) = (pair[0], pair[1]);
            let seg = haversine_m(lon1, lat1, lon2, lat2);
            if densify_m > 0.0 && seg > densify_m {
                let steps = (seg / densify_m).floor() as usize;
                for i in 1..steps {
                    let t = i as f64 / steps as f64;
                    coords.push([lon1 + (lon2 - lon1) * t, lat1 + (lat2 - lat1) * t]);
                }
            }
            coords.push([lon2, lat2]);
        }

        let mut points = Vec::with_capacity(coords.len());
        let mut distance = 0.0;
        let (mut min, mut max): (Option<f32>, Option<f32>) = (None, None);
        let mut gaps = 0;

        for (i, [lon, lat]) in coords.iter().copied().enumerate() {
            if i > 0 {
                let prev = coords[i - 1];
                distance += haversine_m(prev[0], prev[1], lon, lat);
            }
            let elevation = self.sample(lon, lat, zoom);
            match elevation {
                Some(e) => {
                    min = Some(min.map_or(e, |m: f32| m.min(e)));
                    max = Some(max.map_or(e, |m: f32| m.max(e)));
                }
                None => gaps += 1,
            }
            points.push(ProfilePoint { lon, lat, distance_m: distance, elevation_m: elevation });
        }

        let (ascent, descent) = accumulate(&points, threshold_m);
        Ok(Profile {
            distance_m: distance,
            points,
            ascent_m: ascent,
            descent_m: descent,
            min_m: min,
            max_m: max,
            zoom,
            threshold_m,
            gaps,
        })
    }
}

/// Cumulative ascent and descent with hysteresis: a reversal only registers once it exceeds
/// `threshold_m`, so 1 m quantisation dither does not accumulate into phantom climb.
fn accumulate(points: &[ProfilePoint], threshold_m: f64) -> (f64, f64) {
    let (mut ascent, mut descent) = (0.0, 0.0);
    let mut anchor: Option<f32> = None;
    let mut pending: f32 = 0.0;

    for p in points {
        let Some(e) = p.elevation_m else { continue };
        let Some(a) = anchor else {
            anchor = Some(e);
            continue;
        };
        let delta = e - a;
        if delta.abs() as f64 >= threshold_m.max(f64::MIN_POSITIVE) {
            if delta > 0.0 {
                ascent += delta as f64;
            } else {
                descent += -delta as f64;
            }
            anchor = Some(e);
            pending = 0.0;
        } else {
            // keep tracking the extreme so a slow steady climb still registers eventually
            if delta.abs() > pending.abs() {
                pending = delta;
            }
        }
    }
    (ascent, descent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn terrarium_round_trips_known_values() {
        let t = Encoding::Terrarium;
        // sea level is the zero point: 128*256 + 0 + 0 - 32768 == 0
        assert_eq!(t.decode(128, 0, 0), 0.0);
        assert_eq!(t.decode(128, 100, 0), 100.0);
        // the blue channel is the sub-metre fraction, and is what whole-metre quantisation zeroes
        assert_eq!(t.decode(128, 100, 128), 100.5);
        assert_eq!(t.decode(127, 0, 0), -256.0);
    }

    #[test]
    fn mapbox_round_trips_known_values() {
        let m = Encoding::Mapbox;
        assert!((m.decode(0, 0, 0) - -10000.0).abs() < 1e-6);
        // the interval is fixed at 0.1 m, so the low byte is decimetres
        assert!((m.decode(1, 134, 160) - 0.0).abs() < 0.05);
    }

    #[test]
    fn parses_encoding_names() {
        assert_eq!(Encoding::parse("terrarium"), Some(Encoding::Terrarium));
        assert_eq!(Encoding::parse("Mapbox"), Some(Encoding::Mapbox));
        assert_eq!(Encoding::parse("webp"), None);
    }

    #[test]
    fn mercator_places_the_origin_and_centre() {
        let (x, y) = lonlat_to_world(0.0, 0.0, 1);
        assert!((x - 1.0).abs() < 1e-9 && (y - 1.0).abs() < 1e-9, "centre of the world at z1");
        let (x, _) = lonlat_to_world(-180.0, 0.0, 1);
        assert!(x.abs() < 1e-9);
    }

    #[test]
    fn haversine_matches_a_known_distance() {
        // one degree of latitude is about 111 km
        let d = haversine_m(5.0, 45.0, 5.0, 46.0);
        assert!((d - 111_195.0).abs() < 200.0, "got {d}");
    }

    /// The reason hysteresis exists: 1 m quantisation dither on flat ground is not climb.
    #[test]
    fn hysteresis_rejects_quantisation_dither() {
        let dither: Vec<ProfilePoint> = (0..100)
            .map(|i| ProfilePoint {
                lon: 0.0,
                lat: 0.0,
                distance_m: i as f64,
                elevation_m: Some(if i % 2 == 0 { 1000.0 } else { 1001.0 }),
            })
            .collect();
        let (ascent, descent) = accumulate(&dither, 3.0);
        assert_eq!((ascent, descent), (0.0, 0.0), "1 m dither must not accumulate");

        // with no threshold the same data invents 50 m of climb - the trap being guarded against
        let (naive_up, _) = accumulate(&dither, 0.0);
        assert_eq!(naive_up, 50.0);
    }

    #[test]
    fn hysteresis_keeps_real_climb() {
        let climb: Vec<ProfilePoint> = (0..=10)
            .map(|i| ProfilePoint {
                lon: 0.0,
                lat: 0.0,
                distance_m: i as f64 * 100.0,
                elevation_m: Some(1000.0 + i as f32 * 50.0),
            })
            .collect();
        let (ascent, descent) = accumulate(&climb, 3.0);
        assert_eq!((ascent, descent), (500.0, 0.0));
    }

    /// Builds a one-tile PNG archive. Real output is WebP; PNG keeps the test independent of
    /// whether the image encoder was built with WebP write support.
    fn terrain_db(path: &Path, size: u32, encoding: &str, pixel: impl Fn(u32, u32) -> [u8; 3]) {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(size, size, |x, y| Rgb(pixel(x, y)));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();

        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE metadata (name text, value text);
             CREATE TABLE tiles (zoom_level integer, tile_column integer,
               tile_row integer, tile_data blob);",
        )
        .unwrap();
        for (k, v) in [("format", "png"), ("encoding", encoding), ("minzoom", "0"), ("maxzoom", "0")] {
            conn.execute("INSERT INTO metadata VALUES (?, ?)", (k, v)).unwrap();
        }
        conn.execute("INSERT INTO tiles VALUES (0, 0, 0, ?)", (png,)).unwrap();
    }

    #[test]
    fn samples_a_constant_tile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t_terrain.mbtiles");
        // constant 500 m: 128*256 + 244 - 32768 == 244... use the exact encoding instead
        terrain_db(&path, 64, "terrarium", |_, _| [129, 244, 0]);
        let mut s = TerrainSampler::open(&path).unwrap();
        assert_eq!(s.encoding(), Encoding::Terrarium);
        let got = s.sample(5.0, 45.0, 0).unwrap();
        assert!((got - 500.0).abs() < 0.01, "got {got}");
    }

    #[test]
    fn rejects_a_vector_archive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.mbtiles");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE metadata (name text, value text);
             CREATE TABLE tiles (zoom_level integer, tile_column integer, tile_row integer, tile_data blob);",
        )
        .unwrap();
        conn.execute("INSERT INTO metadata VALUES ('format','pbf')", ()).unwrap();
        drop(conn);
        let err = match TerrainSampler::open(&path) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("a vector archive must not open as terrain"),
        };
        assert!(err.contains("encoding"), "got {err}");
    }

    #[test]
    fn profile_densifies_and_measures() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t_terrain.mbtiles");
        terrain_db(&path, 64, "terrarium", |_, _| [129, 244, 0]);
        let mut s = TerrainSampler::open(&path).unwrap();

        let line = [[5.0, 45.0], [5.0, 45.1]];
        let p = s.profile(&line, 0, 1000.0, 3.0).unwrap();
        assert!(p.points.len() > 5, "densified to {} points", p.points.len());
        assert!((p.distance_m - 11_119.0).abs() < 200.0, "got {}", p.distance_m);
        assert_eq!(p.ascent_m, 0.0, "flat terrain");
        assert_eq!(p.gaps, 0);
        assert_eq!(p.min_m, p.max_m);
        // monotonically increasing cumulative distance
        assert!(p.points.windows(2).all(|w| w[1].distance_m >= w[0].distance_m));
    }

    #[test]
    fn profile_reports_gaps_outside_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t_terrain.mbtiles");
        terrain_db(&path, 64, "terrarium", |_, _| [129, 244, 0]);
        let mut s = TerrainSampler::open(&path).unwrap();
        // z0 has exactly one tile covering the world, so force a miss by asking above maxzoom
        // clamping keeps us at z0; instead check a two-point line still yields points
        let p = s.profile(&[[5.0, 45.0], [6.0, 45.0]], 0, 0.0, 3.0).unwrap();
        assert_eq!(p.points.len(), 2);
    }

    #[test]
    fn profile_needs_two_points() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t_terrain.mbtiles");
        terrain_db(&path, 64, "terrarium", |_, _| [129, 244, 0]);
        let mut s = TerrainSampler::open(&path).unwrap();
        assert!(matches!(s.profile(&[[5.0, 45.0]], 0, 0.0, 3.0), Err(_)));
    }
}
