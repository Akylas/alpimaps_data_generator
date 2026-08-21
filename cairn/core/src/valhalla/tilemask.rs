//! Decoding the base64 quadtree tilemask that selects which graph tiles a package carries.
//!
//! The mask is a bitstream walked depth-first over a web-mercator quadtree. Each node spends two
//! bits: the first says whether four children follow, the second whether the node itself is
//! inside the area. A node marked inside with no children means the whole subtree is inside.

use anyhow::{anyhow, Result};
use base64::Engine;

/// One quadtree node's coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuadTile {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

struct BitReader {
    bits: Vec<u8>,
    at: usize,
}

impl BitReader {
    fn next(&mut self) -> Result<u8> {
        let bit = *self
            .bits
            .get(self.at)
            .ok_or_else(|| anyhow!("tilemask ended early at bit {}", self.at))?;
        self.at += 1;
        Ok(bit)
    }
}

/// Expand a mask into the mercator tiles it covers, descending to `max_zoom`.
pub fn tiles(mask: &str, max_zoom: u8) -> Result<Vec<QuadTile>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(mask.trim())
        .map_err(|e| anyhow!("tilemask is not base64: {e}"))?;
    let bits: Vec<u8> = bytes
        .iter()
        .flat_map(|byte| (0..8).map(move |i| (byte >> (7 - i)) & 1))
        .collect();
    if bits.is_empty() {
        return Ok(Vec::new());
    }

    let mut reader = BitReader { bits, at: 0 };
    let mut out = Vec::new();
    walk(&mut reader, QuadTile { z: 0, x: 0, y: 0 }, max_zoom, &mut out)?;
    Ok(out)
}

fn walk(reader: &mut BitReader, tile: QuadTile, max_zoom: u8, out: &mut Vec<QuadTile>) -> Result<()> {
    let has_children = reader.next()? == 1;
    let inside = reader.next()? == 1;
    if inside {
        out.push(tile);
    }
    if has_children {
        for dy in 0..2 {
            for dx in 0..2 {
                let child = QuadTile { z: tile.z + 1, x: tile.x * 2 + dx, y: tile.y * 2 + dy };
                walk(reader, child, max_zoom, out)?;
            }
        }
    } else if inside && tile.z < max_zoom {
        // an inside leaf stands for its whole subtree, so fill it in down to max_zoom
        for dy in 0..2 {
            for dx in 0..2 {
                fill(QuadTile { z: tile.z + 1, x: tile.x * 2 + dx, y: tile.y * 2 + dy }, max_zoom, out);
            }
        }
    }
    Ok(())
}

fn fill(tile: QuadTile, max_zoom: u8, out: &mut Vec<QuadTile>) {
    if tile.z > max_zoom {
        return;
    }
    out.push(tile);
    for dy in 0..2 {
        for dx in 0..2 {
            fill(QuadTile { z: tile.z + 1, x: tile.x * 2 + dx, y: tile.y * 2 + dy }, max_zoom, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(bits: &[u8]) -> String {
        let mut bytes = vec![0u8; bits.len().div_ceil(8)];
        for (i, bit) in bits.iter().enumerate() {
            bytes[i / 8] |= bit << (7 - i % 8);
        }
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn empty_mask_yields_nothing() {
        assert!(tiles("", 3).unwrap().is_empty());
    }

    /// A root marked outside with no children covers nothing.
    #[test]
    fn outside_root_covers_nothing() {
        assert!(tiles(&encode(&[0, 0]), 3).unwrap().is_empty());
    }

    /// A root marked inside with no children stands for the entire pyramid below it.
    #[test]
    fn inside_leaf_expands_to_max_zoom() {
        let got = tiles(&encode(&[0, 1]), 1).unwrap();
        assert_eq!(got.len(), 5, "root plus its four children");
        assert!(got.contains(&QuadTile { z: 0, x: 0, y: 0 }));
        assert!(got.contains(&QuadTile { z: 1, x: 1, y: 1 }));
    }

    #[test]
    fn descends_into_explicit_children() {
        // root has children and is itself outside; first child inside, rest outside
        let bits = [1, 0, /*c0*/ 0, 1, /*c1*/ 0, 0, /*c2*/ 0, 0, /*c3*/ 0, 0];
        let got = tiles(&encode(&bits), 1).unwrap();
        assert_eq!(got, vec![QuadTile { z: 1, x: 0, y: 0 }]);
    }

    #[test]
    fn truncated_mask_is_an_error() {
        // claims children but supplies none
        assert!(tiles(&encode(&[1, 1]), 2).is_err());
    }

    #[test]
    fn rejects_non_base64() {
        assert!(tiles("!!!not base64!!!", 2).is_err());
    }
}
