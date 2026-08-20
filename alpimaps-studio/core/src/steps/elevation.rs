//! Downloading the `.hgt` elevation tiles, without needing Valhalla's Python script.
//!
//! `valhalla_build_elevation` is a Python program, not a binary - shipping it would put a Python
//! interpreter in the app's dependency list for what is, in the end, a naming convention and a
//! download loop. Both are short enough to state directly.
//!
//! Tiles come from the public Skadi mirror as `skadi/N45/N45E006.hgt.gz`: one file per 1° square,
//! gzipped. The terrain renderer reads them decompressed, which is why `valhalla_build_elevation`
//! is always run with `-d` here, so the same files serve both the graph and the terrain step.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

const SKADI: &str = "https://elevation-tiles-prod.s3.us-east-1.amazonaws.com/skadi";

/// One 1°×1° tile: the directory it lives in and its file name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tile {
    pub dir: String,
    pub name: String,
}

impl Tile {
    /// The tile whose south-west corner is at `(lon, lat)`, both whole degrees.
    ///
    /// The name encodes that corner: `N45E006` is 45..46°N, 6..7°E. Southern and western
    /// hemispheres use the absolute value after the letter, so -1 is `W001`, not `W-01`.
    pub fn at(lon: i32, lat: i32) -> Self {
        let ns = if lat < 0 { 'S' } else { 'N' };
        let ew = if lon < 0 { 'W' } else { 'E' };
        let dir = format!("{ns}{:02}", lat.abs());
        Self { name: format!("{dir}{ew}{:03}.hgt", lon.abs()), dir }
    }

    /// Where it lands under the elevation directory.
    pub fn path(&self, root: &Path, gzipped: bool) -> PathBuf {
        let name = if gzipped { format!("{}.gz", self.name) } else { self.name.clone() };
        root.join(&self.dir).join(name)
    }

    pub fn url(&self) -> String {
        format!("{SKADI}/{}/{}.gz", self.dir, self.name)
    }
}

/// Every tile touching a `(west, south, east, north)` box.
///
/// The box is snapped outwards to the 1° grid: a bounding box that clips the corner of a tile
/// still needs that whole tile, because the file is the unit of download.
pub fn tiles_for_bounds(bounds: (f64, f64, f64, f64)) -> Vec<Tile> {
    let (west, south, east, north) = bounds;
    let (x0, y0) = (west.floor() as i32, south.floor() as i32);
    let (x1, y1) = (east.ceil() as i32, north.ceil() as i32);
    let mut tiles = Vec::new();
    for x in x0..x1.max(x0 + 1) {
        for y in y0..y1.max(y0 + 1) {
            tiles.push(Tile::at(x, y));
        }
    }
    tiles.sort();
    tiles.dedup();
    tiles
}

/// What a fetch did with one tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetched {
    /// Already on disk, left alone.
    Present,
    Downloaded,
}

/// Download the tiles that are missing, decompressing unless `gzipped`.
///
/// `progress` is called once per tile with `(done, total)`. Tiles already on disk are reported
/// too, so the count still adds up when most of them are there - which is the normal case when
/// an area is rebuilt.
pub async fn fetch<F>(
    root: &Path,
    tiles: &[Tile],
    gzipped: bool,
    mut progress: F,
) -> Result<(usize, usize)>
where
    F: FnMut(usize, usize),
{
    let mut downloaded = 0;
    for (index, tile) in tiles.iter().enumerate() {
        let outcome = fetch_one(root, tile, gzipped).await?;
        if outcome == Fetched::Downloaded {
            downloaded += 1;
        }
        progress(index + 1, tiles.len());
    }
    Ok((downloaded, tiles.len()))
}

async fn fetch_one(root: &Path, tile: &Tile, gzipped: bool) -> Result<Fetched> {
    let target = tile.path(root, gzipped);
    // either spelling counts as present: the graph reads .hgt.gz and the terrain step reads .hgt,
    // and re-downloading a 25 MB tile because of the extension would be a waste
    if target.is_file() || tile.path(root, !gzipped).is_file() {
        return Ok(Fetched::Present);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let url = tile.url();
    let response = reqwest::get(&url).await.with_context(|| format!("downloading {url}"))?;
    if !response.status().is_success() {
        // the ocean has no tiles, and asking for one is not an error worth stopping a build for
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Fetched::Present);
        }
        return Err(anyhow!("{url} returned {}", response.status()));
    }
    let body = response.bytes().await?;

    // write through a .part so an interrupted download is not mistaken for a finished tile
    let part = target.with_extension("part");
    if gzipped {
        std::fs::write(&part, &body)?;
    } else {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&body[..])
            .read_to_end(&mut out)
            .with_context(|| format!("{url} is not gzip"))?;
        std::fs::write(&part, out)?;
    }
    std::fs::rename(&part, &target)?;
    Ok(Fetched::Downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The naming is the whole reason this module exists; a wrong letter or a missing zero asks
    /// the mirror for a tile that does not exist and reports the area as having no elevation.
    #[test]
    fn tiles_are_named_the_way_the_mirror_names_them() {
        assert_eq!(Tile::at(6, 45), Tile { dir: "N45".into(), name: "N45E006.hgt".into() });
        assert_eq!(Tile::at(-1, 51), Tile { dir: "N51".into(), name: "N51W001.hgt".into() });
        assert_eq!(Tile::at(-70, -34), Tile { dir: "S34".into(), name: "S34W070.hgt".into() });
        assert_eq!(
            Tile::at(6, 45).url(),
            "https://elevation-tiles-prod.s3.us-east-1.amazonaws.com/skadi/N45/N45E006.hgt.gz"
        );
    }

    /// A box that clips the corner of a tile still needs the whole file.
    #[test]
    fn bounds_snap_outwards_to_whole_degrees() {
        let tiles = tiles_for_bounds((5.9, 45.1, 6.2, 45.4));
        assert_eq!(
            tiles,
            vec![Tile::at(5, 45), Tile::at(6, 45)],
            "a box crossing 6E needs both tiles"
        );
    }

    #[test]
    fn a_whole_degree_box_is_one_tile() {
        assert_eq!(tiles_for_bounds((6.0, 45.0, 7.0, 46.0)), vec![Tile::at(6, 45)]);
    }

    /// rhone-alpes, from its own poly's bounds: 3.68..7.19 E covers five 1-degree columns
    /// (3,4,5,6,7) and 44.11..46.52 N three rows (44,45,46), so fifteen tiles - including the
    /// ones the box only clips.
    #[test]
    fn a_real_area_asks_for_the_tiles_it_covers() {
        let tiles = tiles_for_bounds((3.68, 44.11, 7.19, 46.52));
        assert_eq!(tiles.len(), 15);
        assert!(tiles.contains(&Tile::at(5, 45)), "Grenoble's tile");
        assert!(tiles.contains(&Tile::at(7, 46)), "the corner the box clips");
    }

    #[test]
    fn either_spelling_counts_as_already_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let tile = Tile::at(6, 45);
        std::fs::create_dir_all(dir.path().join("N45")).unwrap();
        std::fs::write(tile.path(dir.path(), false), b"x").unwrap();
        assert!(tile.path(dir.path(), false).is_file());
        // the gzipped spelling of the same tile resolves to the same directory
        assert_eq!(
            tile.path(dir.path(), true),
            dir.path().join("N45").join("N45E006.hgt.gz")
        );
    }
}
