//! Reading SRTM `.hgt` elevation tiles.
//!
//! The format is deliberately trivial: a square grid of big-endian `i16` metres, one degree on a
//! side, north row first, west column first, with the last row/column repeating the neighbouring
//! tile's first. Size follows from the file length - 3601 for 1-arcsecond, 1201 for 3-arcsecond.
//! `-32768` marks a void.
//!
//! Because it is geographic (EPSG:4326) on a regular grid, sampling needs no projection library:
//! longitude and latitude index the grid directly. That is what makes the terrain pipeline
//! portable to Rust without pulling in GDAL or PROJ.

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const VOID: i16 = -32768;

pub struct HgtTile {
    /// Samples per side, including the duplicated edge.
    pub size: usize,
    data: Vec<i16>,
}

impl HgtTile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let samples = bytes.len() / 2;
        let size = (samples as f64).sqrt().round() as usize;
        if size * size * 2 != bytes.len() {
            return Err(anyhow!("{} bytes is not a square i16 grid", bytes.len()));
        }
        let data = bytes
            .chunks_exact(2)
            .map(|pair| i16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        Ok(Self { size, data })
    }

    /// Row 0 is the north edge, column 0 the west edge.
    fn at(&self, row: usize, col: usize) -> Option<f32> {
        let value = *self.data.get(row * self.size + col)?;
        (value != VOID).then_some(value as f32)
    }
}

/// The `.hgt` file name for the degree square containing a point, e.g. `N44E006.hgt`.
pub fn file_name(lon_deg: i32, lat_deg: i32) -> String {
    let (ns, lat) = if lat_deg < 0 { ('S', -lat_deg) } else { ('N', lat_deg) };
    let (ew, lon) = if lon_deg < 0 { ('W', -lon_deg) } else { ('E', lon_deg) };
    format!("{ns}{lat:02}{ew}{lon:03}.hgt")
}

pub struct HgtSource {
    root: PathBuf,
    cache: HashMap<(i32, i32), Option<Arc<HgtTile>>>,
}

impl HgtSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), cache: HashMap::new() }
    }

    /// Files sit either directly under the root or in a `N44/` style subdirectory, which is how
    /// the elevation tiles this repository downloads are laid out.
    fn locate(&self, lon_deg: i32, lat_deg: i32) -> Option<PathBuf> {
        let name = file_name(lon_deg, lat_deg);
        let group = &name[..3];
        for candidate in [self.root.join(group).join(&name), self.root.join(&name)] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    fn tile(&mut self, lon_deg: i32, lat_deg: i32) -> Option<Arc<HgtTile>> {
        if let Some(hit) = self.cache.get(&(lon_deg, lat_deg)) {
            return hit.clone();
        }
        let loaded = self
            .locate(lon_deg, lat_deg)
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| HgtTile::parse(&bytes).ok())
            .map(Arc::new);
        self.cache.insert((lon_deg, lat_deg), loaded.clone());
        loaded
    }

    /// The degree square holding a point, with a fallback for exact boundaries.
    ///
    /// A point at lat 45.0 floors into N45, but it is equally the north edge of N44 - and if
    /// only N44 exists on disk, flooring alone loses the sample. Neighbouring `.hgt` tiles
    /// duplicate their shared row and column, so either tile answers identically.
    fn square_for(&mut self, lon: f64, lat: f64) -> Option<(i32, i32, Arc<HgtTile>)> {
        let (lon_deg, lat_deg) = (lon.floor() as i32, lat.floor() as i32);
        let mut candidates = vec![(lon_deg, lat_deg)];
        if lat == lat.floor() {
            candidates.push((lon_deg, lat_deg - 1));
        }
        if lon == lon.floor() {
            candidates.push((lon_deg - 1, lat_deg));
        }
        if lat == lat.floor() && lon == lon.floor() {
            candidates.push((lon_deg - 1, lat_deg - 1));
        }
        for (lo, la) in candidates {
            if let Some(tile) = self.tile(lo, la) {
                return Some((lo, la, tile));
            }
        }
        None
    }

    /// Bilinear sample.
    ///
    /// Indices are clamped inside the covering tile rather than chased into the neighbour: SRTM
    /// repeats the shared edge row and column, so the clamped value is the neighbour's value.
    pub fn sample(&mut self, lon: f64, lat: f64) -> Option<f32> {
        let (lon_deg, lat_deg, tile) = self.square_for(lon, lat)?;
        let last = (tile.size - 1) as f64;

        let fx = ((lon - lon_deg as f64) * last).clamp(0.0, last);
        // row counts south from the north edge, so latitude is inverted
        let fy = ((1.0 - (lat - lat_deg as f64)) * last).clamp(0.0, last);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(tile.size - 1), (y0 + 1).min(tile.size - 1));
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);

        let v00 = tile.at(y0, x0)?;
        let v10 = tile.at(y0, x1).unwrap_or(v00);
        let v01 = tile.at(y1, x0).unwrap_or(v00);
        let v11 = tile.at(y1, x1).unwrap_or(v10);

        let top = v00 + (v10 - v00) * tx;
        let bottom = v01 + (v11 - v01) * tx;
        Some(top + (bottom - top) * ty)
    }

    /// Degree squares present under the root, as `(lon, lat)` pairs.
    pub fn coverage(&self) -> Vec<(i32, i32)> {
        let mut found = Vec::new();
        let mut walk = |dir: &Path| {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if let Some(pair) = parse_name(&name) {
                        found.push(pair);
                    }
                }
            }
        };
        walk(&self.root);
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    walk(&entry.path());
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }
}

fn parse_name(name: &str) -> Option<(i32, i32)> {
    let stem = name.strip_suffix(".hgt")?;
    if stem.len() < 7 {
        return None;
    }
    let lat: i32 = stem[1..3].parse().ok()?;
    let lon: i32 = stem[4..7].parse().ok()?;
    let lat = if stem.starts_with('S') { -lat } else { lat };
    let lon = if stem.as_bytes()[3] == b'W' { -lon } else { lon };
    Some((lon, lat))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(size: usize, f: impl Fn(usize, usize) -> i16) -> Vec<u8> {
        let mut out = Vec::with_capacity(size * size * 2);
        for row in 0..size {
            for col in 0..size {
                out.extend_from_slice(&f(row, col).to_be_bytes());
            }
        }
        out
    }

    #[test]
    fn names_follow_the_srtm_convention() {
        assert_eq!(file_name(6, 44), "N44E006.hgt");
        assert_eq!(file_name(-3, -9), "S09W003.hgt");
        assert_eq!(file_name(0, 0), "N00E000.hgt");
    }

    #[test]
    fn parses_names_back() {
        assert_eq!(parse_name("N44E006.hgt"), Some((6, 44)));
        assert_eq!(parse_name("S09W003.hgt"), Some((-3, -9)));
        assert_eq!(parse_name("readme.txt"), None);
    }

    #[test]
    fn infers_size_from_length() {
        assert_eq!(HgtTile::parse(&grid(1201, |_, _| 0)).unwrap().size, 1201);
        assert_eq!(HgtTile::parse(&grid(3601, |_, _| 0)).unwrap().size, 3601);
        assert!(HgtTile::parse(&[0, 0, 0]).is_err());
    }

    /// Row 0 is the *north* edge. Getting this inverted flips the terrain vertically, which is
    /// subtle enough in hillshade to survive a casual look.
    #[test]
    fn row_zero_is_north() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("N44E006.hgt"),
            // 100 m along the north edge, 200 m along the south
            grid(3, |row, _| if row == 0 { 100 } else if row == 2 { 200 } else { 150 }),
        )
        .unwrap();
        let mut src = HgtSource::new(dir.path());
        assert_eq!(src.sample(6.5, 44.999).unwrap().round(), 100.0, "north edge");
        assert_eq!(src.sample(6.5, 44.001).unwrap().round(), 200.0, "south edge");
    }

    #[test]
    fn voids_read_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("N44E006.hgt"), grid(3, |_, _| VOID)).unwrap();
        let mut src = HgtSource::new(dir.path());
        assert_eq!(src.sample(6.5, 44.5), None);
    }

    #[test]
    fn missing_tile_is_none_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = HgtSource::new(dir.path());
        assert_eq!(src.sample(6.5, 44.5), None);
    }

    /// The repository stores tiles in `N44/` subdirectories, so both layouts must work.
    #[test]
    fn finds_tiles_in_degree_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("N44")).unwrap();
        std::fs::write(dir.path().join("N44/N44E006.hgt"), grid(3, |_, _| 500)).unwrap();
        let mut src = HgtSource::new(dir.path());
        assert_eq!(src.sample(6.5, 44.5).unwrap(), 500.0);
        assert_eq!(src.coverage(), vec![(6, 44)]);
    }

    /// Exactly on a shared edge the point floors into the neighbouring square; SRTM duplicates
    /// that row, so the tile that does exist must still answer.
    #[test]
    fn samples_exactly_on_a_tile_boundary() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("N44E006.hgt"), grid(3, |_, _| 700)).unwrap();
        let mut src = HgtSource::new(dir.path());
        assert_eq!(src.sample(6.5, 45.0), Some(700.0), "north edge, N45 absent");
        assert_eq!(src.sample(7.0, 44.5), Some(700.0), "east edge, E007 absent");
        assert_eq!(src.sample(7.0, 45.0), Some(700.0), "corner");
    }

    #[test]
    fn interpolates_between_posts() {
        let dir = tempfile::tempdir().unwrap();
        // west 0 m, east 200 m, linear across three posts
        std::fs::write(dir.path().join("N44E006.hgt"), grid(3, |_, col| (col * 100) as i16)).unwrap();
        let mut src = HgtSource::new(dir.path());
        let mid = src.sample(6.25, 44.5).unwrap();
        assert!((mid - 50.0).abs() < 1.0, "got {mid}");
    }
}
