//! Building a `.vtiles` routing package from a Valhalla tile directory.
//!
//! Ports `scripts/build_valhalla_package.py`. Two things in that script are load-bearing and
//! carried over deliberately:
//!
//! * **zopfli**, not zlib. Measured -3.03% on rhone-alpes (212.4 MiB against 219.0 MiB) for a
//!   much slower compress and no reader change at all - the output is ordinary gzip.
//! * **`PRAGMA page_size=4096`**. At the SQLite default of 512 bytes a ~1 MB graph tile spans
//!   thousands of overflow pages, each spending four bytes on a next-page pointer.

use super::GraphTile;
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Slower, ~3% smaller, and still plain gzip on the wire.
    Zopfli,
    /// zlib level 9 with a gzip wrapper.
    Zlib,
}

/// Compress one tile to gzip.
pub fn compress(data: &[u8], how: Compression) -> Result<Vec<u8>> {
    match how {
        Compression::Zopfli => {
            let mut out = Vec::new();
            zopfli::compress(
                zopfli::Options::default(),
                zopfli::Format::Gzip,
                data,
                &mut out,
            )?;
            Ok(out)
        }
        Compression::Zlib => {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(9));
            encoder.write_all(data)?;
            Ok(encoder.finish()?)
        }
    }
}

#[derive(Debug, Clone)]
pub struct PackageOptions {
    pub package_id: String,
    pub tile_dir: PathBuf,
    pub output: PathBuf,
    pub compression: Compression,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PackageReport {
    pub tiles_written: usize,
    pub tiles_missing: usize,
    pub raw_bytes: u64,
    pub compressed_bytes: u64,
}

impl PackageReport {
    pub fn ratio(&self) -> f64 {
        if self.raw_bytes == 0 {
            return 0.0;
        }
        self.compressed_bytes as f64 / self.raw_bytes as f64
    }
}

/// Which `.gph` files exist for a set of graph tiles.
pub fn resolve(tile_dir: &Path, tiles: &[GraphTile]) -> (Vec<(GraphTile, PathBuf)>, Vec<GraphTile>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for tile in tiles {
        let path = tile_dir.join(tile.path());
        if path.is_file() {
            found.push((*tile, path));
        } else {
            missing.push(*tile);
        }
    }
    (found, missing)
}

/// Compress and write a package. `on_progress` receives `(done, total)`.
pub fn build<F>(opts: &PackageOptions, tiles: &[GraphTile], mut on_progress: F) -> Result<PackageReport>
where
    F: FnMut(usize, usize),
{
    use rayon::prelude::*;

    let (found, missing) = resolve(&opts.tile_dir, tiles);
    if opts.output.exists() {
        std::fs::remove_file(&opts.output)?;
    }
    if let Some(parent) = opts.output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let total = found.len();
    let how = opts.compression;
    // compression dominates the runtime and every tile is independent
    let compressed: Vec<Result<(GraphTile, u64, Vec<u8>)>> = found
        .par_iter()
        .map(|(tile, path)| {
            let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let blob = compress(&raw, how)?;
            Ok((*tile, raw.len() as u64, blob))
        })
        .collect();

    let conn = Connection::open(&opts.output)?;
    conn.execute_batch(
        "PRAGMA locking_mode=EXCLUSIVE;
         PRAGMA synchronous=OFF;
         PRAGMA page_size=4096;
         PRAGMA encoding='UTF-8';",
    )?;
    conn.execute_batch(
        "CREATE TABLE metadata (name TEXT, value TEXT);
         CREATE TABLE tiles (zoom_level INTEGER, tile_column INTEGER,
           tile_row INTEGER, tile_data BLOB);",
    )?;
    for (name, value) in [
        ("name", opts.package_id.as_str()),
        ("type", "routing"),
        ("version", "1.0"),
        ("format", "gph3"),
    ] {
        conn.execute("INSERT INTO metadata(name, value) VALUES (?, ?)", (name, value))?;
    }
    conn.execute(
        "INSERT INTO metadata(name, value) VALUES ('description', ?)",
        (format!("Nutiteq Valhalla routing package for {}", opts.package_id),),
    )?;

    let mut report = PackageReport { tiles_missing: missing.len(), ..Default::default() };
    {
        let mut stmt = conn.prepare(
            "INSERT INTO tiles(zoom_level, tile_column, tile_row, tile_data) VALUES (?, ?, ?, ?)",
        )?;
        for (done, entry) in compressed.into_iter().enumerate() {
            let (tile, raw_len, blob) = entry?;
            report.raw_bytes += raw_len;
            report.compressed_bytes += blob.len() as u64;
            stmt.execute((tile.level, tile.x, tile.y, &blob))?;
            report.tiles_written += 1;
            on_progress(done + 1, total);
        }
    }
    conn.execute_batch("CREATE UNIQUE INDEX tiles_index ON tiles (zoom_level, tile_column, tile_row);")?;
    drop(conn);

    // VACUUM needs its own connection with no open statements
    Connection::open(&opts.output)?.execute_batch("VACUUM")?;
    Ok(report)
}

/// Expand a package back into a Valhalla tile directory.
///
/// This is what makes the package testable: routing against `valhalla_tiles/` proves the graph
/// builds, while routing against an unpacked `.vtiles` proves the artefact that actually ships.
/// Returns the number of tiles written.
pub fn unpack(package: &Path, tile_dir: &Path) -> Result<usize> {
    use std::io::Read;

    let conn = Connection::open_with_flags(package, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {}", package.display()))?;
    let mut stmt =
        conn.prepare("SELECT zoom_level, tile_column, tile_row, tile_data FROM tiles")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            GraphTile::new(r.get::<_, i64>(1)? as u32, r.get::<_, i64>(2)? as u32, r.get::<_, i64>(0)? as u8),
            r.get::<_, Vec<u8>>(3)?,
        ))
    })?;

    let mut written = 0;
    for row in rows {
        let (tile, blob) = row?;
        let path = tile_dir.join(tile.path());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // tiles are stored gzipped; Valhalla reads them raw from disk
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(&blob[..])
            .read_to_end(&mut raw)
            .with_context(|| format!("decompressing {}", tile.path()))?;
        std::fs::write(&path, raw)?;
        written += 1;
    }
    Ok(written)
}

/// Read the graph tiles listed in an existing package.
pub fn tiles_in(path: &Path) -> Result<Vec<GraphTile>> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt =
        conn.prepare("SELECT zoom_level, tile_column, tile_row FROM tiles ORDER BY 1, 2, 3")?;
    let rows = stmt.query_map([], |r| {
        Ok(GraphTile::new(r.get::<_, i64>(1)? as u32, r.get::<_, i64>(2)? as u32, r.get::<_, i64>(0)? as u8))
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_compressors_produce_readable_gzip() {
        let data = b"valhalla graph tile payload, repeated. ".repeat(400);
        for how in [Compression::Zlib, Compression::Zopfli] {
            let blob = compress(&data, how).unwrap();
            assert_eq!(&blob[..2], &[0x1f, 0x8b], "{how:?} must emit a gzip header");
            let mut out = Vec::new();
            use std::io::Read;
            flate2::read::GzDecoder::new(&blob[..]).read_to_end(&mut out).unwrap();
            assert_eq!(out, data, "{how:?} round trip");
        }
    }

    /// The reason zopfli is the default: same format, smaller output, no reader change.
    #[test]
    fn zopfli_beats_zlib_on_realistic_data() {
        let data: Vec<u8> = (0..60_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        let zlib = compress(&data, Compression::Zlib).unwrap().len();
        let zopfli = compress(&data, Compression::Zopfli).unwrap().len();
        assert!(zopfli <= zlib, "zopfli {zopfli} should not exceed zlib {zlib}");
    }

    #[test]
    fn resolve_separates_present_from_missing() {
        let dir = tempfile::tempdir().unwrap();
        let tile = GraphTile::new(45, 33, 0);
        let path = dir.path().join(tile.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"graph").unwrap();

        let (found, missing) =
            resolve(dir.path(), &[tile, GraphTile::new(1, 1, 0)]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, tile);
        assert_eq!(missing, vec![GraphTile::new(1, 1, 0)]);
    }

    #[test]
    fn builds_a_readable_package() {
        let dir = tempfile::tempdir().unwrap();
        let tile_dir = dir.path().join("tiles");
        let tiles = [GraphTile::new(45, 33, 0), GraphTile::new(5, 3, 1)];
        for tile in tiles {
            let path = tile_dir.join(tile.path());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"graph tile bytes ".repeat(50)).unwrap();
        }

        let output = dir.path().join("area.vtiles");
        let opts = PackageOptions {
            package_id: "area".into(),
            tile_dir,
            output: output.clone(),
            compression: Compression::Zlib,
        };
        let report = build(&opts, &tiles, |_, _| {}).unwrap();
        assert_eq!(report.tiles_written, 2);
        assert_eq!(report.tiles_missing, 0);
        assert!(report.ratio() < 1.0);

        // the package must look exactly like the ones already in use
        let conn = Connection::open(&output).unwrap();
        let format: String = conn
            .query_row("SELECT value FROM metadata WHERE name='format'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(format, "gph3");
        assert_eq!(tiles_in(&output).unwrap().len(), 2);

        let blob: Vec<u8> = conn
            .query_row("SELECT tile_data FROM tiles WHERE zoom_level=0", [], |r| r.get(0))
            .unwrap();
        assert_eq!(&blob[..2], &[0x1f, 0x8b]);
    }

    /// A package must round-trip back to the directory layout Valhalla reads.
    #[test]
    fn unpacks_back_to_a_tile_directory() {
        let dir = tempfile::tempdir().unwrap();
        let tile_dir = dir.path().join("tiles");
        let tiles = [GraphTile::new(45, 33, 0), GraphTile::new(5, 3, 1)];
        let payload = b"graph tile bytes ".repeat(50);
        for tile in tiles {
            let path = tile_dir.join(tile.path());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &payload).unwrap();
        }
        let output = dir.path().join("area.vtiles");
        build(
            &PackageOptions {
                package_id: "area".into(),
                tile_dir: tile_dir.clone(),
                output: output.clone(),
                compression: Compression::Zlib,
            },
            &tiles,
            |_, _| {},
        )
        .unwrap();

        let restored = dir.path().join("restored");
        assert_eq!(unpack(&output, &restored).unwrap(), 2);
        for tile in tiles {
            let got = std::fs::read(restored.join(tile.path())).unwrap();
            assert_eq!(got, payload, "{} did not round-trip", tile.path());
        }
    }

    #[test]
    fn missing_tiles_are_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let opts = PackageOptions {
            package_id: "area".into(),
            tile_dir: dir.path().join("empty"),
            output: dir.path().join("area.vtiles"),
            compression: Compression::Zlib,
        };
        let report = build(&opts, &[GraphTile::new(1, 1, 0)], |_, _| {}).unwrap();
        assert_eq!(report.tiles_written, 0);
        assert_eq!(report.tiles_missing, 1);
    }
}
