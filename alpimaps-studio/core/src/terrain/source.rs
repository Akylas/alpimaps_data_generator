//! Composing elevation sources, the way `sources.json` describes them.
//!
//! `sources.json` lists sources **lowest priority first** - the generator's own convention
//! (`build_terrain_rgb.py`: "Sources are listed lowest priority first"). So the repository's
//! file names `tilezen` before `ignrge5`, and the IGN 5 m raster is the one that should win
//! wherever it has data, with the 1-arcsecond `.hgt` tiles filling in beyond the border.
//!
//! Reading that order the other way round is silent and plausible: every sample still returns a
//! sensible elevation, just from the coarser source, and the output looks fine until compared.
//!
//! Boundaries are feathered the way the generator does it. Its `composite()` builds
//! `weight = box_blur(valid) * valid` and mixes `out * (1 - weight) + source * weight`, so a
//! higher-priority source fades in *inside* its own coverage: zero at its data boundary, rising
//! to one `blur` metres further in. Multiplying by `valid` truncates the ramp so it never bleeds
//! past the edge, because past the edge there is nothing to fade towards.
//!
//! A box blur of a 0/1 mask is just the valid fraction of the neighbourhood, so the same shape
//! is produced here by probing coverage on a ring around each point rather than by convolving an
//! array - this pipeline samples per point and never materialises the mask. The generator runs
//! its blur twice for a triangular ramp; a single ring is closer to linear, which differs in the
//! middle of the fade and not at either end.

use crate::terrain::{geotiff::GeoTiff, hgt::HgtSource, lambert93};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct SourceSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub path: PathBuf,
    #[serde(default)]
    pub clamp_min: Option<f64>,
}

/// Read a `sources.json`, resolving relative paths against its own directory.
pub fn read_specs(path: &Path) -> Result<Vec<SourceSpec>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let mut specs: Vec<SourceSpec> = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", path.display()))?;
    let root = path.parent().unwrap_or(Path::new("."));
    for spec in &mut specs {
        if spec.path.is_relative() {
            spec.path = root.join(&spec.path);
        }
    }
    Ok(specs)
}

enum Backing {
    /// A projected raster. Currently Lambert-93 only, which is what the IGN data uses.
    Raster { tiff: Box<GeoTiff>, clamp_min: Option<f64> },
    Hgt { source: HgtSource, clamp_min: Option<f64> },
}

pub struct CompositeSource {
    sources: Vec<(String, Backing)>,
}

impl CompositeSource {
    /// Open every source that exists. A missing one is skipped rather than fatal, so a machine
    /// without the 44 GB raster still renders from the fallback.
    pub fn open(specs: &[SourceSpec]) -> Result<(Self, Vec<String>)> {
        let mut sources = Vec::new();
        let mut skipped = Vec::new();
        // kept in file order, lowest priority first, because blending has to accumulate in that
        // direction; the unblended path walks it backwards instead
        for spec in specs.iter() {
            if !spec.path.exists() {
                skipped.push(format!("{} ({} missing)", spec.name, spec.path.display()));
                continue;
            }
            match spec.kind.as_str() {
                "raster" => match GeoTiff::open(&spec.path) {
                    Ok(tiff) => sources.push((
                        spec.name.clone(),
                        Backing::Raster { tiff: Box::new(tiff), clamp_min: spec.clamp_min },
                    )),
                    Err(e) => skipped.push(format!("{}: {e}", spec.name)),
                },
                // `valhalla` is what the pipeline calls a directory of .hgt tiles
                "valhalla" | "hgt" => sources.push((
                    spec.name.clone(),
                    Backing::Hgt {
                        source: HgtSource::new(&spec.path),
                        clamp_min: spec.clamp_min,
                    },
                )),
                other => skipped.push(format!("{}: unsupported type {other}", spec.name)),
            }
        }
        Ok((Self { sources }, skipped))
    }

    /// Source names, highest priority first.
    pub fn names(&self) -> Vec<&str> {
        self.sources.iter().rev().map(|(n, _)| n.as_str()).collect()
    }

    /// Elevation from one source, or `None` where it has no data there.
    fn probe(&mut self, index: usize, lon: f64, lat: f64, target_m_per_px: f64) -> Option<f32> {
        let (_, backing) = self.sources.get_mut(index)?;
        let value = match backing {
            Backing::Raster { tiff, clamp_min } => {
                let (easting, northing) = lambert93::from_lonlat(lon, lat);
                let level = tiff.level_for(target_m_per_px);
                tiff.sample(easting, northing, level).map(|v| clamp(v, *clamp_min))
            }
            Backing::Hgt { source, clamp_min } => {
                source.sample(lon, lat).map(|v| clamp(v, *clamp_min))
            }
        };
        value.filter(|v| v.is_finite())
    }

    /// Fraction of a ring of radius `blur_m` around the point where this source has data.
    ///
    /// Stands in for the generator's box blur of the coverage mask. Zero when the centre itself
    /// is uncovered, matching the `* valid` factor that truncates the ramp at the edge.
    fn coverage_weight(&mut self, index: usize, lon: f64, lat: f64, target: f64, blur_m: f64) -> f32 {
        if self.probe(index, lon, lat, target).is_none() {
            return 0.0;
        }
        if blur_m <= 0.0 {
            return 1.0;
        }
        const RING: usize = 8;
        let dlat = blur_m / 111_320.0;
        let dlon = dlat / lat.to_radians().cos().max(1e-6);

        let mut covered = 1usize;
        for i in 0..RING {
            let angle = std::f64::consts::TAU * i as f64 / RING as f64;
            let probe_lon = lon + dlon * angle.cos();
            let probe_lat = lat + dlat * angle.sin();
            if self.probe(index, probe_lon, probe_lat, target).is_some() {
                covered += 1;
            }
        }
        covered as f32 / (RING + 1) as f32
    }

    /// Sample with feathered boundaries between sources.
    ///
    /// Accumulates lowest priority first, exactly as `composite()` does: the first covering
    /// source establishes a value, and each better source mixes in by its coverage weight.
    /// `blur_m` of zero reduces this to a hard switch.
    pub fn sample_blended(&mut self, lon: f64, lat: f64, target_m_per_px: f64, blur_m: f64) -> Option<f32> {
        let mut out: Option<f32> = None;
        for index in 0..self.sources.len() {
            let Some(value) = self.probe(index, lon, lat, target_m_per_px) else { continue };
            out = Some(match out {
                None => value,
                Some(previous) => {
                    let w = self.coverage_weight(index, lon, lat, target_m_per_px, blur_m);
                    previous * (1.0 - w) + value * w
                }
            });
        }
        out
    }

    /// Sample the highest-priority source covering this point.
    ///
    /// `target_m_per_px` picks which overview of a raster source to read: rendering a z8 tile
    /// from 5 m data would decode thousands of full-resolution tiles per output tile, so the
    /// pyramid is used to keep the read proportional to the output.
    pub fn sample(&mut self, lon: f64, lat: f64, target_m_per_px: f64) -> Option<f32> {
        // backwards, because the list is lowest priority first
        for index in (0..self.sources.len()).rev() {
            if let Some(v) = self.probe(index, lon, lat, target_m_per_px) {
                return Some(v);
            }
        }
        None
    }
}

/// `clamp_min` exists to stop bathymetry and nodata artefacts dragging coastlines below sea
/// level; the pipeline sets it to -10 for the global source.
fn clamp(value: f32, min: Option<f64>) -> f32 {
    match min {
        Some(m) => value.max(m as f32),
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat `.hgt` grid at a constant elevation.
    fn flat(size: usize, metres: i16) -> Vec<u8> {
        let mut out = Vec::with_capacity(size * size * 2);
        for _ in 0..size * size {
            out.extend_from_slice(&metres.to_be_bytes());
        }
        out
    }

    #[test]
    fn reads_the_repository_source_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sources.json");
        std::fs::write(
            &path,
            r#"[
              {"name":"tilezen","type":"valhalla","path":"./elevation_tiles","clamp_min":-10},
              {"name":"ignrge5","type":"raster","path":"work/out/ign.tif"}
            ]"#,
        )
        .unwrap();
        let specs = read_specs(&path).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].clamp_min, Some(-10.0));
        // relative paths resolve against the file, not the process working directory
        assert_eq!(specs[1].path, dir.path().join("work/out/ign.tif"));
    }

    /// A missing raster must not stop the fallback source from working - not every machine has
    /// the 44 GB file.
    #[test]
    fn missing_sources_are_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let specs = vec![
            SourceSpec { name: "ign".into(), kind: "raster".into(),
                         path: dir.path().join("absent.tif"), clamp_min: None },
            SourceSpec { name: "tilezen".into(), kind: "valhalla".into(),
                         path: dir.path().to_path_buf(), clamp_min: Some(-10.0) },
        ];
        let (composite, skipped) = CompositeSource::open(&specs).unwrap();
        assert_eq!(composite.names(), vec!["tilezen"]);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("absent.tif"));
    }

    #[test]
    fn unsupported_types_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let specs = vec![SourceSpec {
            name: "weird".into(), kind: "netcdf".into(),
            path: dir.path().to_path_buf(), clamp_min: None,
        }];
        let (composite, skipped) = CompositeSource::open(&specs).unwrap();
        assert!(composite.names().is_empty());
        assert!(skipped[0].contains("netcdf"));
    }

    #[test]
    fn clamp_min_lifts_below_sea_level_values() {
        assert_eq!(clamp(-40.0, Some(-10.0)), -10.0);
        assert_eq!(clamp(120.0, Some(-10.0)), 120.0);
        assert_eq!(clamp(-40.0, None), -40.0);
    }

    /// The list is lowest-priority-first, so the *last* entry that covers a point answers it.
    #[test]
    fn last_listed_source_has_priority() {
        let dir = tempfile::tempdir().unwrap();
        let specs = vec![
            SourceSpec { name: "low".into(), kind: "valhalla".into(),
                         path: dir.path().to_path_buf(), clamp_min: None },
            SourceSpec { name: "high".into(), kind: "valhalla".into(),
                         path: dir.path().to_path_buf(), clamp_min: None },
        ];
        let (composite, _) = CompositeSource::open(&specs).unwrap();
        assert_eq!(composite.names(), vec!["high", "low"], "highest priority is consulted first");
    }

    /// Two overlapping sources at different elevations: inside the better source's coverage the
    /// blend must reach it fully, and at its edge fall back to the other.
    #[test]
    fn blur_ramps_between_sources() {
        let low = tempfile::tempdir().unwrap();
        let high = tempfile::tempdir().unwrap();
        // low priority covers two degree squares at 100 m
        for name in ["N44E006.hgt", "N44E007.hgt"] {
            std::fs::write(low.path().join(name), flat(9, 100)).unwrap();
        }
        // high priority covers only the western one, at 200 m
        std::fs::write(high.path().join("N44E006.hgt"), flat(9, 200)).unwrap();

        let specs = vec![
            SourceSpec { name: "low".into(), kind: "valhalla".into(),
                         path: low.path().to_path_buf(), clamp_min: None },
            SourceSpec { name: "high".into(), kind: "valhalla".into(),
                         path: high.path().to_path_buf(), clamp_min: None },
        ];
        let (mut c, _) = CompositeSource::open(&specs).unwrap();
        let blur = 2000.0;

        // deep inside the better source: fully its value
        let inside = c.sample_blended(6.5, 44.5, 30.0, blur).unwrap();
        assert!((inside - 200.0).abs() < 0.5, "inside gave {inside}");

        // outside it entirely: the fallback, untouched
        let outside = c.sample_blended(7.5, 44.5, 30.0, blur).unwrap();
        assert!((outside - 100.0).abs() < 0.5, "outside gave {outside}");

        // just inside the eastern edge: partway between the two
        let edge = c.sample_blended(6.999, 44.5, 30.0, blur).unwrap();
        assert!(edge > 100.0 && edge < 200.0, "edge gave {edge}, expected a blend");
    }

    /// Zero blur is the old hard switch, and must stay available.
    #[test]
    fn zero_blur_switches_hard() {
        let low = tempfile::tempdir().unwrap();
        let high = tempfile::tempdir().unwrap();
        std::fs::write(low.path().join("N44E006.hgt"), flat(9, 100)).unwrap();
        std::fs::write(high.path().join("N44E006.hgt"), flat(9, 200)).unwrap();
        let specs = vec![
            SourceSpec { name: "low".into(), kind: "valhalla".into(),
                         path: low.path().to_path_buf(), clamp_min: None },
            SourceSpec { name: "high".into(), kind: "valhalla".into(),
                         path: high.path().to_path_buf(), clamp_min: None },
        ];
        let (mut c, _) = CompositeSource::open(&specs).unwrap();
        assert_eq!(c.sample_blended(6.5, 44.5, 30.0, 0.0), Some(200.0));
    }

    /// The ramp is truncated at the edge: where the better source has no data at all the weight
    /// is zero, so nothing of it leaks outside its coverage.
    #[test]
    fn weight_is_zero_outside_coverage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("N44E006.hgt"), flat(9, 200)).unwrap();
        let specs = vec![SourceSpec { name: "only".into(), kind: "valhalla".into(),
                                      path: dir.path().to_path_buf(), clamp_min: None }];
        let (mut c, _) = CompositeSource::open(&specs).unwrap();
        assert_eq!(c.coverage_weight(0, 20.0, 20.0, 30.0, 2000.0), 0.0);
        assert_eq!(c.coverage_weight(0, 6.5, 44.5, 30.0, 2000.0), 1.0, "deep inside is full weight");
    }

    #[test]
    fn first_covering_source_wins() {
        let dir = tempfile::tempdir().unwrap();
        let size = 3usize;
        let mut grid = Vec::new();
        for _ in 0..size * size {
            grid.extend_from_slice(&700i16.to_be_bytes());
        }
        std::fs::write(dir.path().join("N44E006.hgt"), grid).unwrap();

        let specs = vec![
            SourceSpec { name: "absent-raster".into(), kind: "raster".into(),
                         path: dir.path().join("nope.tif"), clamp_min: None },
            SourceSpec { name: "hgt".into(), kind: "valhalla".into(),
                         path: dir.path().to_path_buf(), clamp_min: None },
        ];
        let (mut composite, _) = CompositeSource::open(&specs).unwrap();
        assert_eq!(composite.sample(6.5, 44.5, 30.0), Some(700.0));
        // outside coverage nothing answers
        assert_eq!(composite.sample(20.0, 20.0, 30.0), None);
    }
}
