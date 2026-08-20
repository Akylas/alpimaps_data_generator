//! A read-only BigTIFF reader, just enough for the IGN elevation raster.
//!
//! That file is 44 GB, 198000x196000 Float32, 256-pixel tiles, ZSTD-compressed with the
//! floating-point predictor, in Lambert-93. GDAL reads it in one line - but linking GDAL
//! reintroduces the C toolchain the rest of this pipeline exists to avoid, so the handful of
//! tags actually needed are parsed here instead.
//!
//! Only what the file uses is supported: BigTIFF, tiled layout, single Float32 band, ZSTD or
//! Deflate or no compression, predictors 1 and 3. Anything else is refused rather than guessed
//! at, because a silently mis-decoded elevation tile looks like terrain.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

// the tags this reader needs
const IMAGE_WIDTH: u16 = 256;
const IMAGE_LENGTH: u16 = 257;
const BITS_PER_SAMPLE: u16 = 258;
const COMPRESSION: u16 = 259;
const PREDICTOR: u16 = 317;
const TILE_WIDTH: u16 = 322;
const TILE_LENGTH: u16 = 323;
const TILE_OFFSETS: u16 = 324;
const TILE_BYTE_COUNTS: u16 = 325;
const SAMPLE_FORMAT: u16 = 339;
const MODEL_PIXEL_SCALE: u16 = 33550;
const MODEL_TIEPOINT: u16 = 33922;
const GDAL_NODATA: u16 = 42113;

const COMPRESSION_NONE: u64 = 1;
const COMPRESSION_DEFLATE: u64 = 8;
const COMPRESSION_ADOBE_DEFLATE: u64 = 32946;
const COMPRESSION_ZSTD: u64 = 50000;

#[derive(Debug, Clone)]
enum Value {
    Ints(Vec<u64>),
    Doubles(Vec<f64>),
    Text(String),
}

impl Value {
    fn first_int(&self) -> Option<u64> {
        match self {
            Value::Ints(v) => v.first().copied(),
            Value::Doubles(v) => v.first().map(|d| *d as u64),
            Value::Text(_) => None,
        }
    }
    fn ints(&self) -> Vec<u64> {
        match self {
            Value::Ints(v) => v.clone(),
            _ => Vec::new(),
        }
    }
    fn doubles(&self) -> Vec<f64> {
        match self {
            Value::Doubles(v) => v.clone(),
            _ => Vec::new(),
        }
    }
}

/// One resolution level: the full image, or one of its overviews.
#[derive(Debug, Clone)]
pub struct Level {
    pub width: u64,
    pub height: u64,
    pub tile_width: u64,
    pub tile_height: u64,
    pub compression: u64,
    pub predictor: u64,
    /// Behind an `Arc` because `Level` is cloned per sample and these are large: the
    /// full-resolution level of the IGN raster has 592,884 tiles, so cloning the vectors
    /// outright copied ~9.5 MB for every pixel rendered.
    offsets: std::sync::Arc<Vec<u64>>,
    byte_counts: std::sync::Arc<Vec<u64>>,
    /// Metres per pixel in projected units.
    pub scale: (f64, f64),
    /// Projected coordinate of the top-left corner.
    pub origin: (f64, f64),
}

impl Level {
    /// How many tiles this level is stored in.
    pub fn tile_count(&self) -> usize {
        self.offsets.len()
    }

    pub fn tiles_across(&self) -> u64 {
        self.width.div_ceil(self.tile_width)
    }

    /// Projected coordinate of a pixel centre.
    pub fn pixel_to_proj(&self, px: f64, py: f64) -> (f64, f64) {
        (self.origin.0 + (px + 0.5) * self.scale.0, self.origin.1 - (py + 0.5) * self.scale.1)
    }

    /// Fractional pixel coordinates of a projected point.
    pub fn proj_to_pixel(&self, easting: f64, northing: f64) -> (f64, f64) {
        (
            (easting - self.origin.0) / self.scale.0 - 0.5,
            (self.origin.1 - northing) / self.scale.1 - 0.5,
        )
    }
}

pub struct GeoTiff {
    file: File,
    pub path: PathBuf,
    pub levels: Vec<Level>,
    pub nodata: Option<f64>,
    cache: HashMap<(usize, u64), Option<std::sync::Arc<Vec<f32>>>>,
    cache_order: Vec<(usize, u64)>,
    cache_limit: usize,
}

struct Reader {
    file: File,
    big: bool,
    little_endian: bool,
}

impl Reader {
    fn read_ints(&self, bytes: &[u8], kind: u16, count: usize) -> Vec<u64> {
        let size = type_size(kind);
        (0..count)
            .filter_map(|i| {
                let at = i * size;
                let slice = bytes.get(at..at + size)?;
                Some(match kind {
                    1 => slice[0] as u64,
                    3 => self.u16(slice) as u64,
                    4 => self.u32(slice) as u64,
                    16 | 18 => self.u64(slice),
                    _ => return None,
                })
            })
            .collect()
    }

    fn u16(&self, b: &[u8]) -> u16 {
        let a = [b[0], b[1]];
        if self.little_endian { u16::from_le_bytes(a) } else { u16::from_be_bytes(a) }
    }
    fn u32(&self, b: &[u8]) -> u32 {
        let a = [b[0], b[1], b[2], b[3]];
        if self.little_endian { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) }
    }
    fn u64(&self, b: &[u8]) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&b[..8]);
        if self.little_endian { u64::from_le_bytes(a) } else { u64::from_be_bytes(a) }
    }
    fn f64(&self, b: &[u8]) -> f64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&b[..8]);
        if self.little_endian { f64::from_le_bytes(a) } else { f64::from_be_bytes(a) }
    }
}

fn type_size(kind: u16) -> usize {
    match kind {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 | 16 | 17 | 18 => 8,
        _ => 0,
    }
}

impl GeoTiff {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut header = [0u8; 16];
        file.read_exact(&mut header)?;
        let little_endian = match &header[..2] {
            b"II" => true,
            b"MM" => false,
            other => bail!("not a TIFF: byte order {other:?}"),
        };
        let magic = if little_endian {
            u16::from_le_bytes([header[2], header[3]])
        } else {
            u16::from_be_bytes([header[2], header[3]])
        };
        let big = match magic {
            43 => true,
            42 => false,
            other => bail!("unknown TIFF magic {other}"),
        };

        let mut reader = Reader { file: file.try_clone()?, big, little_endian };
        let mut next = if big {
            reader.u64(&header[8..16])
        } else {
            reader.u32(&header[4..8]) as u64
        };

        let mut levels = Vec::new();
        let mut nodata = None;
        // the IFD chain is the full image followed by its overviews, coarser each step
        while next != 0 && levels.len() < 32 {
            let (tags, following) = read_ifd(&mut reader, next)?;
            if let Some(Value::Text(text)) = tags.get(&GDAL_NODATA) {
                nodata = nodata.or_else(|| text.trim_end_matches('\0').trim().parse().ok());
            }
            if let Some(level) = level_from(&tags, levels.first()) {
                levels.push(level);
            }
            next = following;
        }
        if levels.is_empty() {
            bail!("no tiled image found in {}", path.display());
        }

        file.seek(SeekFrom::Start(0))?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            levels,
            nodata,
            cache: HashMap::new(),
            cache_order: Vec::new(),
            cache_limit: 64,
        })
    }

    /// Decode one tile into a flat grid of `f32`, with nodata mapped to NaN.
    fn tile(&mut self, level_index: usize, tile_index: u64) -> Option<std::sync::Arc<Vec<f32>>> {
        if let Some(hit) = self.cache.get(&(level_index, tile_index)) {
            return hit.clone();
        }
        let decoded = self.decode_tile(level_index, tile_index).ok().flatten().map(std::sync::Arc::new);

        // a 256-square Float32 tile is 256 KB; the cap keeps a full-pyramid render bounded
        if self.cache_order.len() >= self.cache_limit {
            if let Some(oldest) = self.cache_order.first().copied() {
                self.cache.remove(&oldest);
                self.cache_order.remove(0);
            }
        }
        self.cache.insert((level_index, tile_index), decoded.clone());
        self.cache_order.push((level_index, tile_index));
        decoded
    }

    fn decode_tile(&mut self, level_index: usize, tile_index: u64) -> Result<Option<Vec<f32>>> {
        let level = self.levels.get(level_index).ok_or_else(|| anyhow!("no such level"))?.clone();
        let at = tile_index as usize;
        let (Some(&offset), Some(&length)) = (level.offsets.get(at), level.byte_counts.get(at))
        else {
            return Ok(None);
        };
        // a zero-length tile is a legitimate sparse hole, not an error
        if length == 0 {
            return Ok(None);
        }

        let mut raw = vec![0u8; length as usize];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut raw)?;

        let expected = (level.tile_width * level.tile_height * 4) as usize;
        let mut bytes = match level.compression {
            COMPRESSION_NONE => raw,
            COMPRESSION_ZSTD => zstd::stream::decode_all(&raw[..])?,
            COMPRESSION_DEFLATE | COMPRESSION_ADOBE_DEFLATE => {
                let mut out = Vec::with_capacity(expected);
                flate2::read::ZlibDecoder::new(&raw[..]).read_to_end(&mut out)?;
                out
            }
            other => bail!("unsupported TIFF compression {other}"),
        };
        if bytes.len() < expected {
            bytes.resize(expected, 0);
        }

        match level.predictor {
            1 => {}
            3 => undo_float_predictor(&mut bytes, level.tile_width as usize, level.tile_height as usize),
            other => bail!("unsupported TIFF predictor {other}"),
        }

        let nodata = self.nodata;
        let values = bytes[..expected]
            .chunks_exact(4)
            .map(|c| {
                let v = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                match nodata {
                    // nodata becomes NaN so it propagates instead of reading as sea level
                    Some(n) if (v as f64 - n).abs() < 1e-6 => f32::NAN,
                    _ => v,
                }
            })
            .collect();
        Ok(Some(values))
    }

    /// Bilinear sample at a projected coordinate. `level` 0 is full resolution.
    pub fn sample(&mut self, easting: f64, northing: f64, level_index: usize) -> Option<f32> {
        let level = self.levels.get(level_index)?.clone();
        let (fx, fy) = level.proj_to_pixel(easting, northing);
        if fx < 0.0 || fy < 0.0 || fx >= level.width as f64 || fy >= level.height as f64 {
            return None;
        }
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);

        let mut at = |px: f64, py: f64| -> Option<f32> {
            let px = px.clamp(0.0, level.width as f64 - 1.0) as u64;
            let py = py.clamp(0.0, level.height as f64 - 1.0) as u64;
            let tile_index = (py / level.tile_height) * level.tiles_across() + px / level.tile_width;
            let grid = self.tile(level_index, tile_index)?;
            let within = (py % level.tile_height) * level.tile_width + px % level.tile_width;
            let v = *grid.get(within as usize)?;
            v.is_finite().then_some(v)
        };

        let v00 = at(x0, y0)?;
        let v10 = at(x0 + 1.0, y0).unwrap_or(v00);
        let v01 = at(x0, y0 + 1.0).unwrap_or(v00);
        let v11 = at(x0 + 1.0, y0 + 1.0).unwrap_or(v10);
        let top = v00 + (v10 - v00) * tx;
        let bottom = v01 + (v11 - v01) * tx;
        Some(top + (bottom - top) * ty)
    }

    /// The level whose resolution is closest to, but no coarser than, `target` metres per pixel.
    pub fn level_for(&self, target_m_per_px: f64) -> usize {
        let mut best = 0;
        for (i, level) in self.levels.iter().enumerate() {
            if level.scale.0 <= target_m_per_px {
                best = i;
            }
        }
        best
    }

    /// Projected bounds of the full-resolution image.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let l = &self.levels[0];
        let (x0, y0) = (l.origin.0, l.origin.1);
        (x0, y0 - l.height as f64 * l.scale.1, x0 + l.width as f64 * l.scale.0, y0)
    }
}

fn read_ifd(reader: &mut Reader, at: u64) -> Result<(HashMap<u16, Value>, u64)> {
    let entry_size = if reader.big { 20 } else { 12 };
    let count_size = if reader.big { 8 } else { 2 };

    let mut count_buf = vec![0u8; count_size];
    reader.file.seek(SeekFrom::Start(at))?;
    reader.file.read_exact(&mut count_buf)?;
    let entries = if reader.big { reader.u64(&count_buf) } else { reader.u16(&count_buf) as u64 };
    if entries > 4096 {
        bail!("implausible IFD entry count {entries}");
    }

    let mut block = vec![0u8; entries as usize * entry_size];
    reader.file.read_exact(&mut block)?;
    let mut next_buf = vec![0u8; count_size.max(4)];
    let next = {
        let want = if reader.big { 8 } else { 4 };
        next_buf.resize(want, 0);
        reader.file.read_exact(&mut next_buf)?;
        if reader.big { reader.u64(&next_buf) } else { reader.u32(&next_buf) as u64 }
    };

    let inline = if reader.big { 8 } else { 4 };
    let mut tags = HashMap::new();
    for i in 0..entries as usize {
        let e = &block[i * entry_size..(i + 1) * entry_size];
        let tag = reader.u16(&e[0..2]);
        let kind = reader.u16(&e[2..4]);
        let n = if reader.big { reader.u64(&e[4..12]) } else { reader.u32(&e[4..8]) as u64 };
        let payload = &e[if reader.big { 12 } else { 8 }..];

        let size = type_size(kind);
        if size == 0 || n > 1_000_000 {
            continue;
        }
        let total = size * n as usize;
        // small values live inside the entry; larger ones are pointed at
        let bytes = if total <= inline {
            payload[..total.min(payload.len())].to_vec()
        } else {
            let offset = if reader.big { reader.u64(payload) } else { reader.u32(payload) as u64 };
            let mut buf = vec![0u8; total];
            let here = reader.file.stream_position()?;
            reader.file.seek(SeekFrom::Start(offset))?;
            let read = reader.file.read_exact(&mut buf);
            reader.file.seek(SeekFrom::Start(here))?;
            if read.is_err() {
                continue;
            }
            buf
        };

        let value = match kind {
            2 => Value::Text(String::from_utf8_lossy(&bytes).to_string()),
            12 => Value::Doubles(bytes.chunks_exact(8).map(|c| reader.f64(c)).collect()),
            _ => Value::Ints(reader.read_ints(&bytes, kind, n as usize)),
        };
        tags.insert(tag, value);
    }
    Ok((tags, next))
}

fn level_from(tags: &HashMap<u16, Value>, full: Option<&Level>) -> Option<Level> {
    let width = tags.get(&IMAGE_WIDTH)?.first_int()?;
    let height = tags.get(&IMAGE_LENGTH)?.first_int()?;
    let tile_width = tags.get(&TILE_WIDTH)?.first_int()?;
    let tile_height = tags.get(&TILE_LENGTH)?.first_int()?;
    let offsets = tags.get(&TILE_OFFSETS)?.ints();
    let byte_counts = tags.get(&TILE_BYTE_COUNTS)?.ints();
    if offsets.is_empty() || offsets.len() != byte_counts.len() {
        return None;
    }
    if tags.get(&BITS_PER_SAMPLE).and_then(|v| v.first_int()) != Some(32) {
        return None;
    }
    if tags.get(&SAMPLE_FORMAT).and_then(|v| v.first_int()).unwrap_or(3) != 3 {
        return None;
    }

    let scale = tags.get(&MODEL_PIXEL_SCALE).map(|v| v.doubles()).unwrap_or_default();
    let tiepoint = tags.get(&MODEL_TIEPOINT).map(|v| v.doubles()).unwrap_or_default();

    // overviews carry no geo tags of their own; they cover the same ground as the full image, so
    // their scale follows from the size ratio
    let (scale, origin) = if scale.len() >= 2 && tiepoint.len() >= 6 {
        ((scale[0], scale[1]), (tiepoint[3], tiepoint[4]))
    } else if let Some(base) = full {
        let ratio = base.width as f64 / width as f64;
        ((base.scale.0 * ratio, base.scale.1 * ratio), base.origin)
    } else {
        return None;
    };

    Some(Level {
        width,
        height,
        tile_width,
        tile_height,
        compression: tags.get(&COMPRESSION).and_then(|v| v.first_int()).unwrap_or(1),
        predictor: tags.get(&PREDICTOR).and_then(|v| v.first_int()).unwrap_or(1),
        offsets: std::sync::Arc::new(offsets),
        byte_counts: std::sync::Arc::new(byte_counts),
        scale,
        origin,
    })
}

/// Undo TIFF predictor 3, the floating-point predictor, in place.
///
/// It works per row in two stages: a byte-wise cumulative sum, then a de-shuffle from
/// byte-planes back into samples. The plane order is most-significant first regardless of the
/// file's byte order, so the little-endian reassembly reads the planes in reverse.
pub fn undo_float_predictor(bytes: &mut [u8], width: usize, height: usize) {
    const SAMPLE_BYTES: usize = 4;
    let row_bytes = width * SAMPLE_BYTES;

    for row in 0..height {
        let start = row * row_bytes;
        let Some(row_slice) = bytes.get_mut(start..start + row_bytes) else { break };

        // stage 1: bytes are stored as differences from their predecessor
        for i in 1..row_bytes {
            row_slice[i] = row_slice[i].wrapping_add(row_slice[i - 1]);
        }

        // stage 2: regroup byte-planes into samples
        let shuffled = row_slice.to_vec();
        for sample in 0..width {
            for byte in 0..SAMPLE_BYTES {
                row_slice[sample * SAMPLE_BYTES + byte] =
                    shuffled[(SAMPLE_BYTES - byte - 1) * width + sample];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a row the way predictor 3 stores it, so the decoder can be checked against a
    /// known original rather than against itself.
    fn encode_float_predictor(values: &[f32]) -> Vec<u8> {
        let width = values.len();
        let mut planes = vec![0u8; width * 4];
        for (i, v) in values.iter().enumerate() {
            let b = v.to_le_bytes();
            for byte in 0..4 {
                planes[(4 - byte - 1) * width + i] = b[byte];
            }
        }
        // byte-wise differences
        let mut out = planes.clone();
        for i in (1..out.len()).rev() {
            out[i] = out[i].wrapping_sub(out[i - 1]);
        }
        out
    }

    #[test]
    fn float_predictor_round_trips() {
        let values: Vec<f32> = vec![0.0, 1.5, -3.25, 1234.5, 4808.0, -99999.0, 0.125, 42.0];
        let mut encoded = encode_float_predictor(&values);
        undo_float_predictor(&mut encoded, values.len(), 1);
        let decoded: Vec<f32> = encoded
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(decoded, values);
    }

    #[test]
    fn float_predictor_handles_multiple_rows() {
        let row_a: Vec<f32> = (0..16).map(|i| i as f32 * 3.5).collect();
        let row_b: Vec<f32> = (0..16).map(|i| 1000.0 - i as f32).collect();
        let mut buffer = encode_float_predictor(&row_a);
        buffer.extend(encode_float_predictor(&row_b));
        undo_float_predictor(&mut buffer, 16, 2);
        let decoded: Vec<f32> = buffer
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(&decoded[..16], &row_a[..]);
        assert_eq!(&decoded[16..], &row_b[..]);
    }

    #[test]
    fn type_sizes_match_the_spec() {
        assert_eq!(type_size(1), 1);
        assert_eq!(type_size(3), 2);
        assert_eq!(type_size(4), 4);
        assert_eq!(type_size(12), 8);
        assert_eq!(type_size(16), 8);
        assert_eq!(type_size(999), 0, "unknown types are skipped, not guessed");
    }

    #[test]
    fn pixel_and_projected_coordinates_round_trip() {
        let level = Level {
            width: 100, height: 100, tile_width: 16, tile_height: 16,
            compression: 1, predictor: 1,
            offsets: std::sync::Arc::new(vec![]), byte_counts: std::sync::Arc::new(vec![]),
            scale: (5.0, 5.0), origin: (94_997.5, 7_115_002.5),
        };
        let (e, n) = level.pixel_to_proj(10.0, 20.0);
        let (px, py) = level.proj_to_pixel(e, n);
        assert!((px - 10.0).abs() < 1e-9 && (py - 20.0).abs() < 1e-9, "{px},{py}");
    }

    #[test]
    fn rejects_a_non_tiff() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.tif");
        std::fs::write(&path, b"not a tiff at all, really").unwrap();
        assert!(GeoTiff::open(&path).is_err());
    }
}
