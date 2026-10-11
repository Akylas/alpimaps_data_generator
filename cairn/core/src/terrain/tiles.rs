//! Elevation out of a raster tile pyramid - mapterhorn, or anything shaped like it.
//!
//! The other two sources this pipeline reads are whole grids: a `.hgt` degree square, or a
//! GeoTIFF with its overviews. A tiled source is different in kind, because the pyramid already
//! *is* the overviews - so the zoom to read is chosen from the output's ground resolution, the
//! same way [`super::geotiff`] picks a level, and a z7 output tile never decodes a z12 tile.
//!
//! Three places hold tiles, each for a different reason:
//!
//! * decoded elevation grids, in memory, because one output tile samples the same source tile a
//!   quarter of a million times;
//! * the raw encoded tiles, on disk, because the next run of the same area would otherwise ask a
//!   public service for every one of them again;
//! * for a PMTiles archive, its directories, because the archive is addressed by them and they do
//!   not change.
//!
//! The disk cache is not optional. mapterhorn's tiles come from a free public endpoint, and a
//! single alpine build at z5-z12 asks for tens of thousands of them; re-fetching that on every
//! rebuild is the kind of thing that gets a pipeline blocked, and deservedly.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};

use super::fetch::Fetcher;
use super::pmtiles::{self, Entry, Header, TileType};
use crate::elevation::Encoding;

/// Decoded grids kept in memory, per source.
///
/// A 512-square grid is 1 MB of `f32`. An output tile samples the source at a zoom chosen to
/// match its own resolution, so it touches a handful of source tiles and the working set is tiny;
/// the cap only has to cover the run of output tiles a worker walks. It is deliberately modest
/// because there is one of these per render thread, and eighteen of them is the normal case.
const GRID_CACHE: usize = 64;

/// Where a tileset lives.
#[derive(Debug, Clone)]
pub enum Location {
    /// A `.pmtiles` file on disk.
    Local(PathBuf),
    /// A URL: a PMTiles archive read by byte range, or an `{z}/{x}/{y}` template.
    Remote(String),
}

impl Location {
    /// `https://` and `http://` name a remote source; anything else is a path.
    pub fn parse(raw: &str) -> Self {
        if raw.starts_with("https://") || raw.starts_with("http://") {
            Location::Remote(raw.to_string())
        } else {
            Location::Local(PathBuf::from(raw))
        }
    }
}

#[derive(Debug, Clone)]
pub struct TilesConfig {
    pub encoding: Encoding,
    /// Pixels per side. Only used to choose a zoom before the first tile is decoded; after that
    /// the tiles say what they are.
    pub tile_size: u32,
    /// Overrides for the zoom range. A PMTiles archive states its own; an XYZ template cannot.
    pub minzoom: Option<u8>,
    pub maxzoom: Option<u8>,
    /// Where fetched tiles are kept between runs.
    pub cache_dir: Option<PathBuf>,
}

impl Default for TilesConfig {
    fn default() -> Self {
        Self {
            encoding: Encoding::Terrarium,
            tile_size: 512,
            minzoom: None,
            maxzoom: None,
            cache_dir: None,
        }
    }
}

struct Grid {
    size: u32,
    data: Vec<f32>,
}

/// The answer to the previous grid lookup: its key, and what it returned - `None` included,
/// because a tile the source does not have is asked for just as repeatedly as one it does.
type Memo = ((u8, u32, u32), Option<Arc<Grid>>);

/// What one tiled source did over a build. Summed across the render threads before it is shown.
#[derive(Debug, Clone, Default)]
pub struct TileStats {
    pub decoded: u64,
    pub from_cache: u64,
    pub requests: u64,
    pub bytes: u64,
    /// Requests that failed every retry. Each one is a tile missing from the output.
    pub failures: u64,
    /// Tiles that could not be read or decoded, for any reason.
    pub unreadable: u64,
    pub last_error: Option<String>,
}

impl TileStats {
    pub fn merge(&mut self, other: &TileStats) {
        self.decoded += other.decoded;
        self.from_cache += other.from_cache;
        self.requests += other.requests;
        self.bytes += other.bytes;
        self.failures += other.failures;
        self.unreadable += other.unreadable;
        if self.last_error.is_none() {
            self.last_error = other.last_error.clone();
        }
    }
}

impl std::fmt::Display for TileStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} decoded, {} from the tile cache; {} requests, {:.1} MB, {} failed",
            self.decoded,
            self.from_cache,
            self.requests,
            self.bytes as f64 / 1_048_576.0,
            self.failures
        )?;
        if self.unreadable > 0 {
            write!(
                f,
                "; {} tiles unreadable, last: {}",
                self.unreadable,
                self.last_error.as_deref().unwrap_or("")
            )?;
        }
        Ok(())
    }
}

/// Tiles already on disk, from an earlier run or an earlier zoom.
struct DiskCache {
    root: PathBuf,
    /// A cache that cannot be written is a performance problem, not a correctness one, so it is
    /// reported once and then ignored.
    complained: AtomicBool,
}

impl DiskCache {
    fn new(root: PathBuf) -> Self {
        Self { root, complained: AtomicBool::new(false) }
    }

    fn tile_path(&self, z: u8, x: u32, y: u32, extension: &str) -> PathBuf {
        self.root.join("tiles").join(z.to_string()).join(x.to_string()).join(format!("{y}.{extension}"))
    }

    /// Marks a tile the source says does not exist, so the next run does not ask again. The
    /// ocean is mostly this.
    fn absent_path(&self, z: u8, x: u32, y: u32) -> PathBuf {
        self.root.join("tiles").join(z.to_string()).join(x.to_string()).join(format!("{y}.absent"))
    }

    fn meta_path(&self, name: &str) -> PathBuf {
        self.root.join("meta").join(name)
    }

    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    fn store(&self, path: &Path, bytes: &[u8]) {
        if let Err(e) = self.try_store(path, bytes) {
            if !self.complained.swap(true, Ordering::Relaxed) {
                eprintln!("  tile cache at {} is not writable: {e}", self.root.display());
            }
        }
    }

    fn try_store(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Through a .part, so an interrupted write is never read back as a tile - and the .part
        // carries a counter, because several worker threads may fetch the same tile at once and
        // one shared scratch name would have them writing over each other's bytes.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let part = path.with_extension(format!("part{serial}"));
        std::fs::write(&part, bytes)?;
        // rename is atomic within a filesystem, so a reader sees either the old file or the whole
        // new one, never a half-written tile
        std::fs::rename(&part, path)?;
        Ok(())
    }
}

enum Backing {
    Local(Mutex<std::fs::File>),
    Remote { base: String, fetcher: Arc<Fetcher> },
}

struct Reader {
    backing: Backing,
    cache: Option<DiskCache>,
}

impl Reader {
    fn open(location: &Location, cache: Option<DiskCache>) -> Result<Self> {
        let backing = match location {
            Location::Local(path) => Backing::Local(Mutex::new(
                std::fs::File::open(path)
                    .with_context(|| format!("opening {}", path.display()))?,
            )),
            Location::Remote(url) => {
                Backing::Remote { base: url.clone(), fetcher: Arc::new(Fetcher::new()) }
            }
        };
        Ok(Self { backing, cache })
    }

    /// `length` bytes from `offset`. `None` when the source has nothing there.
    fn range(&self, offset: u64, length: u64) -> Result<Option<Vec<u8>>> {
        match &self.backing {
            Backing::Local(file) => {
                let mut file = file.lock().map_err(|_| anyhow!("the archive handle is poisoned"))?;
                file.seek(SeekFrom::Start(offset))?;
                let mut buffer = vec![0u8; length as usize];
                match file.read_exact(&mut buffer) {
                    Ok(()) => Ok(Some(buffer)),
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
                    Err(e) => Err(e.into()),
                }
            }
            Backing::Remote { base, fetcher } => {
                // HTTP ranges are inclusive at both ends
                fetcher.get(base, Some((offset, offset + length - 1)))
            }
        }
    }

    fn fetcher(&self) -> Option<&Fetcher> {
        match &self.backing {
            Backing::Remote { fetcher, .. } => Some(fetcher),
            Backing::Local(_) => None,
        }
    }
}

/// A PMTiles archive's directories, read lazily and kept.
struct PmArchive {
    header: Header,
    root: Vec<Entry>,
    leaves: HashMap<(u64, u64), Arc<Vec<Entry>>>,
}

impl PmArchive {
    fn open(reader: &Reader) -> Result<Self> {
        // the header and the root directory are what every later lookup needs, and for a remote
        // archive they are the only two round trips a build makes before it starts on tiles
        let header_bytes = match reader.cache.as_ref().and_then(|c| c.read(&c.meta_path("header.bin"))) {
            Some(bytes) if bytes.len() >= pmtiles::HEADER_LEN => bytes,
            _ => {
                let bytes = reader
                    .range(0, pmtiles::HEADER_LEN as u64)?
                    .ok_or_else(|| anyhow!("the archive is shorter than a PMTiles header"))?;
                if let Some(cache) = &reader.cache {
                    cache.store(&cache.meta_path("header.bin"), &bytes);
                }
                bytes
            }
        };
        let header = Header::parse(&header_bytes)?;
        if !matches!(header.tile_type, TileType::Webp | TileType::Png | TileType::Jpeg) {
            return Err(anyhow!(
                "{:?} tiles cannot carry elevation; this wants a terrain-RGB raster archive",
                header.tile_type
            ));
        }
        let root = Self::directory(reader, &header, header.root_offset, header.root_length)?;
        Ok(Self { header, root: (*root).clone(), leaves: HashMap::new() })
    }

    fn directory(
        reader: &Reader,
        header: &Header,
        offset: u64,
        length: u64,
    ) -> Result<Arc<Vec<Entry>>> {
        let name = format!("dir-{offset}-{length}.bin");
        if let Some(bytes) = reader.cache.as_ref().and_then(|c| c.read(&c.meta_path(&name))) {
            return Ok(Arc::new(pmtiles::parse_directory(&bytes)?));
        }
        let raw = reader
            .range(offset, length)?
            .ok_or_else(|| anyhow!("the archive has no directory at {offset}"))?;
        let bytes = pmtiles::decompress(header.internal_compression, &raw)?;
        if let Some(cache) = &reader.cache {
            cache.store(&cache.meta_path(&name), &bytes);
        }
        Ok(Arc::new(pmtiles::parse_directory(&bytes)?))
    }

    /// Where a tile's bytes are in the archive, following one leaf directory when the root
    /// defers to it.
    fn locate(&mut self, reader: &Reader, z: u8, x: u32, y: u32) -> Result<Option<(u64, u64)>> {
        let id = pmtiles::zxy_to_tile_id(z, x, y);
        let Some(entry) = pmtiles::find(&self.root, id) else { return Ok(None) };
        let entry = if entry.run_length == 0 {
            let key = (entry.offset, entry.length as u64);
            let leaf = match self.leaves.get(&key) {
                Some(leaf) => leaf.clone(),
                None => {
                    let leaf = Self::directory(
                        reader,
                        &self.header,
                        self.header.leaf_offset + key.0,
                        key.1,
                    )?;
                    self.leaves.insert(key, leaf.clone());
                    leaf
                }
            };
            match pmtiles::find(&leaf, id) {
                // a leaf pointing at another leaf is legal in the format but not written by any
                // tool this pipeline meets, and following it blindly could loop
                Some(found) if found.run_length > 0 => found,
                _ => return Ok(None),
            }
        } else {
            entry
        };
        Ok(Some((self.header.tile_data_offset + entry.offset, entry.length as u64)))
    }
}

enum Layout {
    /// A `{z}/{x}/{y}` URL template.
    Xyz,
    PmTiles(Box<PmArchive>),
}

pub struct TerrainTiles {
    reader: Reader,
    layout: Layout,
    encoding: Encoding,
    minzoom: u8,
    maxzoom: u8,
    tile_size: u32,
    extension: String,
    tile_compression: pmtiles::Compression,
    grids: HashMap<(u8, u32, u32), Option<Arc<Grid>>>,
    /// Insertion order, for evicting the oldest grid once the cap is reached.
    order: VecDeque<(u8, u32, u32)>,
    /// What the last lookup answered, to spare the inner loop a hash of its own.
    last: Option<Memo>,
    from_disk: u64,
    decoded: u64,
    errors: u64,
    last_error: Option<String>,
}

impl TerrainTiles {
    /// An `{z}/{x}/{y}.webp` style template, as mapterhorn's tile endpoint serves.
    pub fn xyz(template: &str, config: TilesConfig) -> Result<Self> {
        if !(template.contains("{z}") && template.contains("{x}") && template.contains("{y}")) {
            return Err(anyhow!("`{template}` is not a {{z}}/{{x}}/{{y}} template"));
        }
        let extension = template
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("webp")
            .to_string();
        let reader = Reader::open(
            &Location::Remote(template.to_string()),
            config.cache_dir.clone().map(DiskCache::new),
        )?;
        Ok(Self {
            reader,
            layout: Layout::Xyz,
            encoding: config.encoding,
            // a template says nothing about its own extent, so the caller's range is all there is
            minzoom: config.minzoom.unwrap_or(0),
            maxzoom: config.maxzoom.unwrap_or(12),
            tile_size: config.tile_size,
            extension,
            tile_compression: pmtiles::Compression::None,
            grids: HashMap::new(),
            order: VecDeque::new(),
            last: None,
            from_disk: 0,
            decoded: 0,
            errors: 0,
            last_error: None,
        })
    }

    /// A PMTiles archive, on disk or read by byte range over HTTP.
    pub fn pmtiles(location: &Location, config: TilesConfig) -> Result<Self> {
        let reader = Reader::open(location, config.cache_dir.clone().map(DiskCache::new))?;
        let archive = PmArchive::open(&reader)?;
        let header = archive.header.clone();
        Ok(Self {
            reader,
            encoding: config.encoding,
            minzoom: config.minzoom.unwrap_or(header.min_zoom),
            maxzoom: config.maxzoom.unwrap_or(header.max_zoom),
            tile_size: config.tile_size,
            extension: header.tile_type.extension().to_string(),
            tile_compression: header.tile_compression,
            layout: Layout::PmTiles(Box::new(archive)),
            grids: HashMap::new(),
            order: VecDeque::new(),
            last: None,
            from_disk: 0,
            decoded: 0,
            errors: 0,
            last_error: None,
        })
    }

    /// The zoom whose pixels are at least as fine as `target_m_per_px`.
    ///
    /// Reading a finer zoom than the output needs is not just slow, it is slow by a factor of
    /// four per level - and over a metered public endpoint it is also rude.
    fn zoom_for(&self, lat: f64, target_m_per_px: f64) -> u8 {
        if target_m_per_px <= 0.0 || target_m_per_px.is_nan() {
            return self.maxzoom;
        }
        // ground resolution at zoom 0 for this latitude
        let base = 40_075_016.686 * lat.to_radians().cos().abs() / self.tile_size as f64;
        let zoom = (base / target_m_per_px).log2().ceil();
        if !zoom.is_finite() {
            return self.maxzoom;
        }
        (zoom.clamp(0.0, 30.0) as u8).clamp(self.minzoom, self.maxzoom)
    }

    /// The encoded bytes of one tile, from disk when they are already there.
    fn blob(&mut self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>> {
        if let Some(cache) = &self.reader.cache {
            if cache.absent_path(z, x, y).exists() {
                return Ok(None);
            }
            if let Some(bytes) = cache.read(&cache.tile_path(z, x, y, &self.extension)) {
                self.from_disk += 1;
                return Ok(Some(bytes));
            }
        }

        // disjoint fields: the layout is borrowed mutably while the reader stays shared
        let fetched = match &mut self.layout {
            Layout::Xyz => {
                let Backing::Remote { base, fetcher } = &self.reader.backing else {
                    return Err(anyhow!("an XYZ template needs a URL"));
                };
                let url = base
                    .replace("{z}", &z.to_string())
                    .replace("{x}", &x.to_string())
                    .replace("{y}", &y.to_string());
                fetcher.get(&url, None)?
            }
            Layout::PmTiles(archive) => match archive.locate(&self.reader, z, x, y)? {
                Some((offset, length)) => self.reader.range(offset, length)?,
                None => None,
            },
        };

        if let Some(cache) = &self.reader.cache {
            match &fetched {
                Some(bytes) => cache.store(&cache.tile_path(z, x, y, &self.extension), bytes),
                // an empty file: the marker is the name, not the contents
                None => cache.store(&cache.absent_path(z, x, y), &[]),
            }
        }
        Ok(fetched)
    }

    fn decode(&self, bytes: &[u8]) -> Result<Grid> {
        let bytes = pmtiles::decompress(self.tile_compression, bytes)?;
        let image = image::load_from_memory(&bytes)?.to_rgb8();
        let (width, height) = image.dimensions();
        if width != height {
            return Err(anyhow!("a {width}x{height} tile is not square"));
        }
        let encoding = self.encoding;
        Ok(Grid {
            size: width,
            data: image.pixels().map(|p| encoding.decode(p[0], p[1], p[2])).collect(),
        })
    }

    fn grid(&mut self, z: u8, x: u32, y: u32) -> Option<Arc<Grid>> {
        // the same key is asked for once per bilinear texel and again for the next pixel, so the
        // last answer is kept beside the map - misses included, because an absent tile is asked
        // for just as repeatedly as a present one
        if let Some((key, grid)) = &self.last {
            if *key == (z, x, y) {
                return grid.clone();
            }
        }
        if let Some(cached) = self.grids.get(&(z, x, y)) {
            let cached = cached.clone();
            self.last = Some(((z, x, y), cached.clone()));
            return cached;
        }
        let loaded = match self.blob(z, x, y) {
            Ok(Some(bytes)) => match self.decode(&bytes) {
                Ok(grid) => {
                    self.decoded += 1;
                    // a tile that decodes tells us the real pixel size, which the caller only
                    // guessed at; zoom choice from here on uses the truth
                    self.tile_size = grid.size;
                    Some(Arc::new(grid))
                }
                Err(e) => {
                    self.note(format!("z{z}/{x}/{y}: {e}"));
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                // a hole here would be silent in the output, so it is counted and reported at the
                // end of the render rather than swallowed
                self.note(format!("z{z}/{x}/{y}: {e}"));
                None
            }
        };
        self.grids.insert((z, x, y), loaded.clone());
        self.last = Some(((z, x, y), loaded.clone()));
        self.order.push_back((z, x, y));
        while self.order.len() > GRID_CACHE {
            if let Some(oldest) = self.order.pop_front() {
                self.grids.remove(&oldest);
            }
        }
        loaded
    }

    fn note(&mut self, message: String) {
        self.errors += 1;
        self.last_error = Some(message);
    }

    fn texel(&mut self, z: u8, world_x: f64, world_y: f64) -> Option<f32> {
        if world_x < 0.0 || world_y < 0.0 {
            return None;
        }
        let size = self.tile_size as f64;
        let (tile_x, tile_y) = ((world_x / size) as u32, (world_y / size) as u32);
        let grid = self.grid(z, tile_x, tile_y)?;
        let stride = grid.size as usize;
        let (px, py) = ((world_x % size) as usize, (world_y % size) as usize);
        grid.data.get(py * stride + px).copied()
    }

    /// Bilinear sample at whichever zoom matches `target_m_per_px`.
    pub fn sample(&mut self, lon: f64, lat: f64, target_m_per_px: f64) -> Option<f32> {
        if !(-85.06..=85.06).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return None;
        }
        let zoom = self.zoom_for(lat, target_m_per_px);
        let count = (1u64 << zoom) as f64;
        let tile_x = (lon + 180.0) / 360.0 * count;
        let radians = lat.to_radians();
        let tile_y = (1.0 - (radians.tan() + 1.0 / radians.cos()).ln() / std::f64::consts::PI)
            / 2.0
            * count;
        if tile_x < 0.0 || tile_y < 0.0 {
            return None;
        }

        // the covering tile is decoded first, because pixel coordinates depend on its size and
        // assuming 512 mislocates every sample in a 256-pixel tileset
        self.grid(zoom, tile_x as u32, tile_y as u32)?;
        let size = self.tile_size as f64;
        let (wx, wy) = (tile_x * size - 0.5, tile_y * size - 0.5);
        let (x0, y0) = (wx.floor(), wy.floor());
        let (fx, fy) = ((wx - x0) as f32, (wy - y0) as f32);

        let v00 = self.texel(zoom, x0, y0)?;
        let v10 = self.texel(zoom, x0 + 1.0, y0).unwrap_or(v00);
        let v01 = self.texel(zoom, x0, y0 + 1.0).unwrap_or(v00);
        let v11 = self.texel(zoom, x0 + 1.0, y0 + 1.0).unwrap_or(v10);

        let top = v00 + (v10 - v00) * fx;
        let bottom = v01 + (v11 - v01) * fx;
        Some(top + (bottom - top) * fy)
    }

    /// What this source did, for the end of a render.
    ///
    /// Counted rather than formatted, because a parallel build has one of these per worker and
    /// eighteen lines of half a story is worse than one line of the whole one.
    pub fn stats(&self) -> TileStats {
        let (requests, bytes, failures) =
            self.reader.fetcher().map(|f| f.stats()).unwrap_or((0, 0, 0));
        TileStats {
            decoded: self.decoded,
            from_cache: self.from_disk,
            requests,
            bytes,
            failures,
            unreadable: self.errors,
            last_error: self.last_error.clone(),
        }
    }

    /// Tiles this source could not read. Non-zero means the archive has holes it should not.
    pub fn errors(&self) -> u64 {
        self.errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::render;

    /// A terrarium tile at one flat elevation, as the encoder would write it.
    fn flat_tile(size: u32, metres: f32) -> Vec<u8> {
        let [r, g, b] = render::encode(Encoding::Terrarium, metres, 0.0);
        let image = image::RgbImage::from_pixel(size, size, image::Rgb([r, g, b]));
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn cache_with(dir: &Path, z: u8, x: u32, y: u32, bytes: &[u8]) {
        let path = dir.join("tiles").join(z.to_string()).join(x.to_string()).join(format!("{y}.png"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn cached_xyz(dir: &Path) -> TerrainTiles {
        TerrainTiles::xyz(
            // deliberately unreachable: anything this test reads must come from the cache
            "https://invalid.invalid/{z}/{x}/{y}.png",
            TilesConfig {
                encoding: Encoding::Terrarium,
                tile_size: 256,
                minzoom: Some(0),
                maxzoom: Some(5),
                cache_dir: Some(dir.to_path_buf()),
            },
        )
        .unwrap()
    }

    /// The point of the disk cache: a second run reads tiles without asking the server again.
    #[test]
    fn cached_tiles_are_read_without_a_request() {
        let dir = tempfile::tempdir().unwrap();
        // z5 covering 6.5E 45.5N
        let (x, y) = (16u32, 11u32);
        cache_with(dir.path(), 5, x, y, &flat_tile(256, 1200.0));

        let mut tiles = cached_xyz(dir.path());
        let sampled = tiles.sample(6.5, 45.5, 1.0).unwrap();
        assert!((sampled - 1200.0).abs() < 0.5, "got {sampled}");
        assert_eq!(tiles.errors(), 0, "{}", tiles.stats());
        let (requests, _, _) = tiles.reader.fetcher().unwrap().stats();
        assert_eq!(requests, 0, "a cached tile must not be fetched");
    }

    /// An `.absent` marker stands for a tile the source has already said it does not have.
    #[test]
    fn absent_markers_stop_the_source_being_asked_twice() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("tiles/5/16/11.absent");
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, []).unwrap();

        let mut tiles = cached_xyz(dir.path());
        assert_eq!(tiles.sample(6.5, 45.5, 1.0), None);
        assert_eq!(tiles.reader.fetcher().unwrap().stats().0, 0);
        assert_eq!(tiles.errors(), 0);
    }

    /// A failed fetch must be counted, not silently turned into empty terrain.
    #[test]
    fn unreachable_tiles_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        let mut tiles = cached_xyz(dir.path());
        assert_eq!(tiles.sample(6.5, 45.5, 1.0), None);
        assert_eq!(tiles.errors(), 1, "{}", tiles.stats());
    }

    /// Zoom follows the output's resolution: a coarse output must not pull the finest tiles.
    #[test]
    fn zoom_matches_the_requested_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let tiles = cached_xyz(dir.path());
        // at the equator z0 is 40075016/256 m per pixel; halving the target adds a level
        let base = 40_075_016.686 / 256.0;
        assert_eq!(tiles.zoom_for(0.0, base), 0);
        assert_eq!(tiles.zoom_for(0.0, base / 2.0), 1);
        assert_eq!(tiles.zoom_for(0.0, base / 8.0), 3);
        // finer than the tileset goes is capped rather than asked for
        assert_eq!(tiles.zoom_for(0.0, 0.01), 5);
    }

    #[test]
    fn templates_are_checked() {
        let err = match TerrainTiles::xyz("https://example.com/tiles.png", TilesConfig::default()) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("a URL without {{z}}/{{x}}/{{y}} is not a template"),
        };
        assert!(err.contains("template"), "{err}");
    }

    #[test]
    fn locations_tell_urls_from_paths() {
        assert!(matches!(Location::parse("https://a/b.pmtiles"), Location::Remote(_)));
        assert!(matches!(Location::parse("/tmp/b.pmtiles"), Location::Local(_)));
    }

    /// Round-trip through a real archive written by the `pmtiles` crate: the reader has to find
    /// the tile the writer put there, which is the whole of the Hilbert and directory code.
    #[test]
    fn reads_a_pmtiles_archive_written_by_the_writer() {
        use ::pmtiles::{Compression, PmTilesWriter, TileCoord, TileType as WriterTileType};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("terrain.pmtiles");
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = PmTilesWriter::new(WriterTileType::Png)
            .tile_compression(Compression::None)
            .min_zoom(0)
            .max_zoom(5)
            .create(file)
            .unwrap();
        // the tile covering 6.5E 45.5N at z5, and one elsewhere so the directory has a choice.
        // A stream writer wants them in tile-id order, which is the reader's order too.
        let mut written = vec![
            (TileCoord::new(5, 16, 11).unwrap(), flat_tile(256, 1200.0)),
            (TileCoord::new(5, 1, 1).unwrap(), flat_tile(256, 40.0)),
        ];
        written.sort_by_key(|(coord, _)| u64::from(::pmtiles::TileId::from(*coord)));
        for (coord, bytes) in &written {
            writer.add_raw_tile(*coord, bytes).unwrap();
        }
        writer.finalize().unwrap();

        let mut tiles = TerrainTiles::pmtiles(
            &Location::Local(path),
            TilesConfig { tile_size: 256, cache_dir: None, ..TilesConfig::default() },
        )
        .unwrap();
        assert_eq!(tiles.maxzoom, 5, "the archive's own zoom range is used");
        let sampled = tiles.sample(6.5, 45.5, 1.0).unwrap();
        assert!((sampled - 1200.0).abs() < 0.5, "got {sampled}");
        // a tile the archive does not hold is no data, not an error
        assert_eq!(tiles.sample(-120.0, 30.0, 1.0), None);
        assert_eq!(tiles.errors(), 0, "{}", tiles.stats());
    }
}
