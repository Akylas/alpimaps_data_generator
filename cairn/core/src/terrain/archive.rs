//! Where rendered terrain tiles go: an MBTiles, or a PMTiles.
//!
//! The terrain step is the one that writes its own archive rather than handing tiles to an
//! external tool, so unlike the planetiler and tippecanoe steps it cannot get PMTiles by naming
//! the output differently. This is that second writer, behind the same three calls, so the
//! render loops do not know or care which one they are feeding.
//!
//! It also ends the duplication that was there before: `create_archive` plus a prepared INSERT
//! plus a final CREATE INDEX existed once in the CLI and once in the Tauri command, and the two
//! had to be kept in step by hand.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pmtiles::{Compression, PmTilesStreamWriter, PmTilesWriter, TileCoord, TileType};
use rusqlite::Connection;

use super::render::TerrainOptions;
use crate::elevation::Encoding;
use crate::steps::archive::ArchiveFormat;

/// One tile archive being written.
pub enum TerrainArchive {
    MBTiles(Box<MBTilesArchive>),
    PMTiles(Box<PMTilesArchive>),
}

impl TerrainArchive {
    /// Create the archive, with the metadata a terrain reader needs.
    pub fn create(
        path: &Path,
        format: ArchiveFormat,
        name: &str,
        opts: &TerrainOptions,
        bounds: (f64, f64, f64, f64),
        image_format: &str,
    ) -> Result<Self> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(match format {
            ArchiveFormat::MBTiles => TerrainArchive::MBTiles(Box::new(MBTilesArchive::create(
                path,
                name,
                opts,
                bounds,
                image_format,
            )?)),
            ArchiveFormat::PMTiles => TerrainArchive::PMTiles(Box::new(PMTilesArchive::create(
                path,
                name,
                opts,
                bounds,
                image_format,
            )?)),
        })
    }

    /// Add one tile, addressed the way the renderer thinks: XYZ, y counting from the north.
    pub fn add(&mut self, z: u8, x: u32, y: u32, data: &[u8]) -> Result<()> {
        match self {
            TerrainArchive::MBTiles(a) => a.add(z, x, y, data),
            TerrainArchive::PMTiles(a) => a.add(z, x, y, data),
        }
    }

    /// Close the archive. Not optional: PMTiles writes nothing usable until this runs.
    pub fn finish(self) -> Result<()> {
        match self {
            TerrainArchive::MBTiles(a) => a.finish(),
            TerrainArchive::PMTiles(a) => a.finish(),
        }
    }
}

fn encoding_name(encoding: Encoding) -> &'static str {
    match encoding {
        Encoding::Terrarium => "terrarium",
        Encoding::Mapbox => "mapbox",
    }
}

/// The metadata pairs both containers carry, in one place so they cannot disagree.
fn metadata_pairs(
    name: &str,
    opts: &TerrainOptions,
    bounds: (f64, f64, f64, f64),
    image_format: &str,
) -> Vec<(&'static str, String)> {
    let encoding = encoding_name(opts.encoding);
    vec![
        ("name", name.to_string()),
        ("format", image_format.to_string()),
        ("type", "baselayer".into()),
        ("version", "1".into()),
        ("description", format!("{encoding} terrain rgb")),
        // MapLibre needs this exact key to decode elevation from the tiles
        ("encoding", encoding.into()),
        ("minzoom", opts.minzoom.to_string()),
        ("maxzoom", opts.maxzoom.to_string()),
        ("bounds", format!("{},{},{},{}", bounds.0, bounds.1, bounds.2, bounds.3)),
    ]
}

pub struct MBTilesArchive {
    conn: Connection,
}

impl MBTilesArchive {
    fn create(
        path: &Path,
        name: &str,
        opts: &TerrainOptions,
        bounds: (f64, f64, f64, f64),
        image_format: &str,
    ) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=OFF;
             PRAGMA synchronous=OFF;
             CREATE TABLE metadata (name text, value text);
             CREATE TABLE tiles (zoom_level integer, tile_column integer,
               tile_row integer, tile_data blob);",
        )?;
        for (key, value) in metadata_pairs(name, opts, bounds, image_format) {
            conn.execute("INSERT INTO metadata VALUES (?, ?)", (key, value))?;
        }
        Ok(Self { conn })
    }

    fn add(&mut self, z: u8, x: u32, y: u32, data: &[u8]) -> Result<()> {
        // mbtiles rows count up from the south; the renderer counts down from the north
        let tms_row = (1u32 << z) - 1 - y;
        self.conn.execute(
            "INSERT INTO tiles VALUES (?, ?, ?, ?)",
            rusqlite::params![z, x, tms_row, data],
        )?;
        Ok(())
    }

    fn finish(self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE UNIQUE INDEX tile_index ON tiles (zoom_level, tile_column, tile_row)",
        )?;
        Ok(())
    }
}

/// A PMTiles archive, assembled at the end from tiles spilled to a temporary file.
///
/// PMTiles addresses tiles by Hilbert id and wants them written in ascending order - a reader
/// fetching a run of neighbouring tiles then gets one contiguous range instead of a scatter.
/// The render loops walk zoom, then column, then row, which is not that order, and holding a
/// whole terrain pyramid in memory to sort it is not an option: a rhone-alpes z5-12 archive is
/// hundreds of megabytes.
///
/// So the tiles go to a scratch file as they are rendered, only their id, offset and length are
/// kept, and `finish` sorts those and copies the bytes back out in order. That is the same
/// shape tippecanoe uses for its own PMTiles output, and it costs one extra pass over the tile
/// bytes plus a transient copy on disk.
pub struct PMTilesArchive {
    path: PathBuf,
    spill_path: PathBuf,
    spill: BufWriter<File>,
    entries: Vec<Spilled>,
    offset: u64,
    writer: PmTilesWriter,
}

struct Spilled {
    id: u64,
    coord: TileCoord,
    offset: u64,
    len: u32,
}

impl PMTilesArchive {
    fn create(
        path: &Path,
        name: &str,
        opts: &TerrainOptions,
        bounds: (f64, f64, f64, f64),
        image_format: &str,
    ) -> Result<Self> {
        let tile_type = match image_format {
            "png" => TileType::Png,
            _ => TileType::Webp,
        };

        // The metadata is JSON here rather than a table. `encoding` has to survive that move or
        // MapLibre cannot decode the elevation, which is the whole point of the archive.
        let metadata: serde_json::Map<String, serde_json::Value> =
            metadata_pairs(name, opts, bounds, image_format)
                .into_iter()
                .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
                .collect();

        let writer = PmTilesWriter::new(tile_type)
            // WebP and PNG are already compressed; gzipping them again costs time and grows
            // the file
            .tile_compression(Compression::None)
            .min_zoom(opts.minzoom)
            .max_zoom(opts.maxzoom)
            .bounds(bounds.0, bounds.1, bounds.2, bounds.3)
            .metadata(&serde_json::Value::Object(metadata).to_string());

        // Read *and* write: the tiles go in here as they are rendered and come back out in
        // tile-id order at the end, so a write-only handle fails with EBADF on the way back.
        let spill_path = path.with_extension("pmtiles.spill");
        let spill = BufWriter::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&spill_path)
                .with_context(|| format!("creating {}", spill_path.display()))?,
        );

        Ok(Self {
            path: path.to_path_buf(),
            spill_path,
            spill,
            entries: Vec::new(),
            offset: 0,
            writer,
        })
    }

    fn add(&mut self, z: u8, x: u32, y: u32, data: &[u8]) -> Result<()> {
        let coord = TileCoord::new(z, x, y)
            .map_err(|e| anyhow::anyhow!("{z}/{x}/{y} is not a valid tile: {e}"))?;
        self.spill.write_all(data)?;
        self.entries.push(Spilled {
            id: u64::from(pmtiles::TileId::from(coord)),
            coord,
            offset: self.offset,
            len: data.len() as u32,
        });
        self.offset += data.len() as u64;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.spill.flush()?;
        let mut spill = self.spill.into_inner().context("flushing the tile scratch file")?;

        self.entries.sort_by_key(|e| e.id);

        let out = File::create(&self.path)
            .with_context(|| format!("creating {}", self.path.display()))?;
        let mut writer: PmTilesStreamWriter<File> = self
            .writer
            .create(out)
            .map_err(|e| anyhow::anyhow!("starting {}: {e}", self.path.display()))?;

        let mut buffer = Vec::new();
        for entry in &self.entries {
            spill.seek(SeekFrom::Start(entry.offset))?;
            buffer.resize(entry.len as usize, 0);
            spill.read_exact(&mut buffer)?;
            // already the final bytes, and the header says so - the writer must not touch them
            writer
                .add_raw_tile(entry.coord, &buffer)
                .map_err(|e| anyhow::anyhow!("writing a tile: {e}"))?;
        }

        writer
            .finalize()
            .map_err(|e| anyhow::anyhow!("finishing {}: {e}", self.path.display()))?;

        drop(spill);
        let _ = std::fs::remove_file(&self.spill_path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> TerrainOptions {
        TerrainOptions { minzoom: 2, maxzoom: 3, encoding: Encoding::Terrarium, ..Default::default() }
    }

    fn sample_tiles() -> Vec<(u8, u32, u32, Vec<u8>)> {
        // deliberately not in Hilbert order, and with one duplicate payload to exercise the
        // writer's deduplication
        vec![
            (3, 5, 2, b"tile-a".to_vec()),
            (2, 1, 1, b"tile-b".to_vec()),
            (3, 1, 1, b"tile-a".to_vec()),
            (2, 0, 0, b"tile-c".to_vec()),
        ]
    }

    #[test]
    fn mbtiles_round_trips_every_tile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.mbtiles");
        let mut archive = TerrainArchive::create(
            &path,
            ArchiveFormat::MBTiles,
            "t",
            &opts(),
            (3.0, 44.0, 7.0, 46.0),
            "webp",
        )
        .unwrap();
        for (z, x, y, data) in sample_tiles() {
            archive.add(z, x, y, &data).unwrap();
        }
        archive.finish().unwrap();

        let conn = Connection::open(&path).unwrap();
        for (z, x, y, data) in sample_tiles() {
            let tms = (1u32 << z) - 1 - y;
            let got: Vec<u8> = conn
                .query_row(
                    "SELECT tile_data FROM tiles WHERE zoom_level=? AND tile_column=? AND tile_row=?",
                    (z, x, tms),
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(got, data, "{z}/{x}/{y}");
        }
        let encoding: String = conn
            .query_row("SELECT value FROM metadata WHERE name='encoding'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(encoding, "terrarium");
    }

    /// The header has to say PMTiles v3, and the tiles have to be there. Anything less and the
    /// step would be writing a file nothing can read - which is exactly what happened when the
    /// bathymap's temporary file hid its extension from tippecanoe.
    #[test]
    fn pmtiles_writes_a_readable_v3_archive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.pmtiles");
        let mut archive = TerrainArchive::create(
            &path,
            ArchiveFormat::PMTiles,
            "t",
            &opts(),
            (3.0, 44.0, 7.0, 46.0),
            "webp",
        )
        .unwrap();
        for (z, x, y, data) in sample_tiles() {
            archive.add(z, x, y, &data).unwrap();
        }
        archive.finish().unwrap();

        // Read the header against the v3 byte layout rather than through the crate that wrote
        // it: a writer and its own reader agreeing proves nothing about what a browser will see.
        let bytes = std::fs::read(&path).unwrap();
        let u64_at = |at: usize| {
            u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
        };

        assert_eq!(&bytes[0..7], b"PMTiles", "magic");
        assert_eq!(bytes[7], 3, "spec version");
        assert_eq!(u64_at(72), 4, "addressed tiles");
        assert_eq!(u64_at(88), 3, "distinct payloads - the duplicate is stored once");
        assert_eq!(bytes[96], 1, "clustered: tiles written in tile-id order");
        assert_eq!(bytes[98], 1, "tile compression: none, the tiles are already WebP");
        assert_eq!(bytes[99], 4, "tile type: WebP");
        assert_eq!(bytes[100], 2, "min zoom");
        assert_eq!(bytes[101], 3, "max zoom");

        // and the metadata a terrain reader actually needs survived the move to JSON
        let meta_off = u64_at(24) as usize;
        let meta_len = u64_at(32) as usize;
        let mut json = Vec::new();
        flate2::read::GzDecoder::new(&bytes[meta_off..meta_off + meta_len])
            .read_to_end(&mut json)
            .expect("metadata is gzipped, per the internal compression byte");
        let meta: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(meta["encoding"], "terrarium");
        assert_eq!(meta["format"], "webp");
    }

    /// The scratch file is transient; leaving it behind would double the archive on disk.
    #[test]
    fn the_spill_file_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.pmtiles");
        let mut archive =
            TerrainArchive::create(&path, ArchiveFormat::PMTiles, "t", &opts(), (3.0, 44.0, 7.0, 46.0), "webp")
                .unwrap();
        archive.add(2, 1, 1, b"x").unwrap();
        archive.finish().unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains("spill"))
            .collect();
        assert!(leftovers.is_empty(), "left behind {leftovers:?}");
    }

    /// Moved here with the writer. It is the check that the archive says enough about itself
    /// for the catalog and the viewer to know what it holds.
    #[test]
    fn archive_metadata_matches_what_the_viewer_needs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t_terrain.mbtiles");
        // named rather than defaulted: this checks that what the options say reaches the
        // metadata, so it must not move when the defaults do
        let opts = TerrainOptions {
            encoding: Encoding::Terrarium,
            maxzoom: 13,
            ..TerrainOptions::default()
        };
        TerrainArchive::create(
            &path,
            ArchiveFormat::MBTiles,
            "t_terrain",
            &opts,
            (3.0, 44.0, 7.0, 46.0),
            "webp",
        )
        .unwrap()
        .finish()
        .unwrap();

        let art = crate::catalog::probe(&path, "t");
        assert_eq!(art.kind, crate::catalog::ArtifactKind::TerrainRgb);
        assert_eq!(art.encoding.as_deref(), Some("terrarium"));
        assert_eq!(art.maxzoom, Some(13));
    }

    /// The catalog reads a PMTiles archive's own metadata now, so it must come back with the
    /// same facts an mbtiles would give - not just the kind its name implies.
    #[test]
    fn a_pmtiles_terrain_archive_is_classified_from_its_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t_terrain.pmtiles");
        let opts = TerrainOptions { minzoom: 4, maxzoom: 7, encoding: Encoding::Terrarium, ..Default::default() };
        let mut archive = TerrainArchive::create(
            &path,
            ArchiveFormat::PMTiles,
            "t_terrain",
            &opts,
            (3.0, 44.0, 7.0, 46.0),
            "webp",
        )
        .unwrap();
        archive.add(4, 8, 5, b"tile").unwrap();
        archive.finish().unwrap();

        let art = crate::catalog::probe(&path, "t");
        assert_eq!(art.probe_error, None, "a PMTiles must not read back as unreadable");
        assert_eq!(art.kind, crate::catalog::ArtifactKind::TerrainRgb);
        assert_eq!(art.encoding.as_deref(), Some("terrarium"), "the DEM encoding has to survive");
        assert_eq!((art.minzoom, art.maxzoom), (Some(4), Some(7)));
        assert_eq!(art.format, crate::catalog::TileFormat::Webp);

        let stats = crate::catalog::tile_stats(&path).unwrap();
        assert_eq!(stats.addressed_tiles, 1);
        assert_eq!(stats.per_zoom, vec![crate::catalog::ZoomStat { zoom: 4, tiles: 1, bytes: 4 }]);
    }

    /// Classification must still work from the name alone, for an archive whose metadata says
    /// nothing useful.
    #[test]
    fn a_pmtiles_terrain_archive_is_still_classified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t_terrain.pmtiles");
        let mut archive = TerrainArchive::create(
            &path,
            ArchiveFormat::PMTiles,
            "t_terrain",
            &opts(),
            (3.0, 44.0, 7.0, 46.0),
            "webp",
        )
        .unwrap();
        archive.add(2, 1, 1, b"tile").unwrap();
        archive.finish().unwrap();

        let art = crate::catalog::probe(&path, "t");
        assert_eq!(art.kind, crate::catalog::ArtifactKind::TerrainRgb);
    }

    /// A small archive fits its whole index in the root directory. Past about 16 kB of entries
    /// the writer has to spill into leaf directories, and that is a different code path with a
    /// different header - one a terrain pyramid reaches easily and the tests above never would.
    #[test]
    fn a_large_archive_uses_leaf_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.pmtiles");
        let opts = TerrainOptions { minzoom: 0, maxzoom: 8, ..opts() };
        let mut archive =
            TerrainArchive::create(&path, ArchiveFormat::PMTiles, "big", &opts, (-180.0, -85.0, 180.0, 85.0), "webp")
                .unwrap();

        // distinct payloads, so nothing deduplicates and the index really does get large
        let mut written = 0u64;
        for x in 0..256u32 {
            for y in 0..64u32 {
                archive.add(8, x, y, format!("tile-{x}-{y}").as_bytes()).unwrap();
                written += 1;
            }
        }
        archive.finish().unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        assert_eq!(&bytes[0..7], b"PMTiles");
        assert_eq!(u64_at(72), written, "addressed tiles");
        assert_eq!(u64_at(88), written, "every payload distinct");
        assert!(u64_at(48) > 0, "leaf directory section is empty, so no leaves were written");
        assert_eq!(bytes[96], 1, "still clustered");
    }
}
