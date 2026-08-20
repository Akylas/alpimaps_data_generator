//! Declarative option schema for build steps.
//!
//! The point is that the GUI never hard-codes a form: it renders whatever these definitions
//! say, and the same definitions turn the collected values back into a command line. That keeps
//! the form and the argv in step, and makes the argv testable without running anything.
//!
//! Every flag name here was read out of the sources rather than remembered - the stock ones from
//! `PlanetilerConfig`, the custom ones from the fork's `Route`/`Landcover` layers. Planetiler's
//! `Arguments` treats `-` and `_` in a flag name as equivalent, so `--simplify-tolerance` and
//! `--simplify_tolerance` reach the same setting.
//!
//! Defaults are deliberately *absent*. An option carries a `hint` describing what planetiler
//! does when the flag is omitted, but the schema never asserts that value - so an unset option
//! emits nothing and planetiler's own default stands, instead of this file's guess about it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OptionKind {
    Bool,
    Int { min: Option<i64>, max: Option<i64> },
    Float { min: Option<f64>, max: Option<f64> },
    Text,
    Choice { choices: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptionDef {
    /// Stable key used in presets and in the values map.
    pub key: String,
    /// Flag as passed to planetiler, without the leading `--`.
    pub flag: String,
    pub label: String,
    pub help: String,
    pub group: String,
    pub kind: OptionKind,
    /// What happens when the option is left unset. Documentation only - never emitted.
    pub hint: String,
}

fn opt(key: &str, flag: &str, label: &str, group: &str, kind: OptionKind, help: &str, hint: &str) -> OptionDef {
    OptionDef {
        key: key.into(),
        flag: flag.into(),
        label: label.into(),
        group: group.into(),
        kind,
        help: help.into(),
        hint: hint.into(),
    }
}

fn float(min: f64) -> OptionKind {
    OptionKind::Float { min: Some(min), max: None }
}

/// Options shared by every planetiler-driven step.
pub fn planetiler_common() -> Vec<OptionDef> {
    vec![
        opt("simplify_tolerance", "simplify-tolerance", "Simplify tolerance", "Geometry",
            float(0.0),
            "Douglas-Peucker tolerance in tile pixels below max zoom. Removes vertices; never \
             deletes a feature, so lowering detail here costs shape fidelity but not content.",
            "planetiler's own default applies"),
        opt("simplify_tolerance_at_max_zoom", "simplify-tolerance-at-max-zoom", "Simplify tolerance (max zoom)", "Geometry",
            float(0.0),
            "Same, applied only at the maximum zoom.",
            "falls back to the value below max zoom"),
        opt("min_feature_size", "min-feature-size", "Min feature size", "Geometry",
            float(0.0),
            "Deletion threshold in tile pixels below max zoom. SQUARED for polygons, so this is \
             a minimum area - raising it drops small polygons entirely rather than simplifying \
             them, which is why it reads as missing forest and grass rather than coarser forest.",
            "planetiler's own default applies"),
        opt("min_feature_size_at_max_zoom", "min-feature-size-at-max-zoom", "Min feature size (max zoom)", "Geometry",
            float(0.0),
            "Same, applied only at the maximum zoom.",
            "falls back to the value below max zoom"),
        opt("maxzoom", "maxzoom", "Max zoom", "Zooms",
            OptionKind::Int { min: Some(0), max: Some(15) },
            "Highest zoom rendered. The top zoom dominates output size - z14 is about 71% of a \
             rhone-alpes basemap.",
            "14"),
        opt("minzoom", "minzoom", "Min zoom", "Zooms",
            OptionKind::Int { min: Some(0), max: Some(15) }, "Lowest zoom rendered.", "0"),
        opt("nodemap_type", "nodemap-type", "Node map", "Performance",
            OptionKind::Choice { choices: vec!["sparsearray".into(), "sortedtable".into(), "array".into()] },
            "How OSM node locations are held. `sparsearray` is the low-memory choice and is what \
             makes a 16 GB machine viable.",
            "planetiler picks based on input size"),
        opt("parallel_tmp_io", "parallel-tmp-io", "Parallel temp IO", "Performance",
            OptionKind::Bool, "Read and write sort chunks in parallel.", "off"),
        opt("compact_db", "compact-db", "Compact archive", "Output",
            OptionKind::Bool,
            "Store each distinct tile blob once behind a `tiles` view. Measured on the current \
             rhone-alpes output this deduplicates only 0.3% of tiles, so the indirection is \
             close to free but also close to pointless.",
            "off"),
        opt("skip_filled_tiles", "skip-filled-tiles", "Skip filled tiles", "Output",
            OptionKind::Bool, "Omit tiles whose content is entirely covered by their parent.", "off"),
        opt("languages", "languages", "Languages", "Output",
            OptionKind::Text,
            "Comma-separated name languages to keep. Empty drops all localised names.",
            "all languages"),
        opt("polygon", "polygon", "Clip polygon", "Output",
            OptionKind::Text, "Path to a .poly clipping the build to a shape.", "the extract's own bbox"),
    ]
}

/// Terrain-RGB options.
///
/// These are the renderer's own knobs, not planetiler flags - `flag` names the CLI flag so the
/// same values can be shown as an `alpimaps terrain` command line. The step used to be handed
/// `TerrainOptions::default()` regardless of what the form said.
pub fn terrain_options() -> Vec<OptionDef> {
    vec![
        opt("minzoom", "minzoom", "Min zoom", "Zooms",
            OptionKind::Int { min: Some(0), max: Some(15) }, "Lowest zoom rendered.", "5"),
        opt("maxzoom", "maxzoom", "Max zoom", "Zooms",
            OptionKind::Int { min: Some(0), max: Some(15) },
            "Highest zoom rendered, and the zoom the quantisation ramp is anchored to.", "13"),
        opt("encoding", "encoding", "Encoding", "Packing",
            OptionKind::Choice { choices: vec!["terrarium".into(), "mapbox".into()] },
            "How elevation is packed into RGB. terrarium is 1 m per step at round-digits 8;              mapbox is 0.1 m, which is what the older `_hillshade` archives use.",
            "terrarium"),
        opt("round_digits", "round-digits", "Round digits", "Packing",
            OptionKind::Int { min: Some(0), max: Some(16) },
            "Quantisation exponent at the maximum zoom. The step is `interval * 2^round_digits`,              so raising it coarsens elevation and compresses far better.",
            "8"),
        opt("max_round_digits", "max-round-digits", "Max round digits", "Packing",
            OptionKind::Int { min: Some(0), max: Some(16) },
            "Cap on the per-zoom ramp: lower zooms quantise more coarsely, up to this.",
            "15"),
        opt("tile_size", "tile-size", "Tile size", "Output",
            OptionKind::Int { min: Some(256), max: Some(1024) }, "Pixels per side.", "512"),
        opt("format", "format", "Format", "Output",
            OptionKind::Choice { choices: vec!["webp".into(), "png".into()] },
            "Lossless WebP is much smaller; PNG is for tools that will not read WebP.", "webp"),
        opt("blur", "blur", "Source blend", "Sources",
            float(0.0),
            "Metres over which a higher-priority source fades in at its coverage boundary, so              the seam between IGN and tilezen data is a ramp rather than a step.",
            "1000"),
        opt("nodata_elevation", "nodata-elevation", "No-data elevation", "Sources",
            OptionKind::Float { min: None, max: None },
            "Elevation written where no source covers a pixel. build_terrain_rgb.py used -10 so              uncovered pixels read as sea; every archive in this repository was built with 0.",
            "0"),
        opt("download_elevation", "no-elevation-download", "Fetch missing tiles", "Sources",
            OptionKind::Bool,
            "Download any .hgt tile this render needs and does not have. A missing tile is \
             otherwise silent: the renderer writes nothing there and the archive comes out with \
             a hole.",
            "on"),
        opt("poly_shape", "poly-shape", "Clip shape", "Sources",
            OptionKind::Text,
            "Path to an osmosis .poly. Only tiles touching the shape are written.",
            "the whole bounding box"),
        opt("tile_buffer", "tile-buffer", "Tile buffer", "Sources",
            OptionKind::Int { min: Some(0), max: Some(8) },
            "Ring of extra tiles around the shape. 3D renderers backfill a DEM tile's 1px border              from its neighbours, so without a ring there is a seam where coverage stops.",
            "0"),
        opt("bounds", "bounds", "Bounds", "Sources",
            OptionKind::Text, "west,south,east,north.",
            "the shape's bounds, else the area's basemap bounds"),
    ]
}

/// Valhalla package options.
pub fn package_options() -> Vec<OptionDef> {
    vec![
        opt("compression", "compression", "Compression", "Output",
            OptionKind::Choice { choices: vec!["zopfli".into(), "zlib".into()] },
            "Both emit ordinary gzip. zopfli is about 3% smaller and much slower.", "zopfli"),
        opt("poly", "poly", "Tile selection shape", "Tiles",
            OptionKind::Text,
            "Path to an osmosis .poly. Every graph tile the shape touches is packed.",
            "the tile list of the package already there"),
        opt("levels", "levels", "Hierarchy levels", "Tiles",
            OptionKind::Text, "Comma-separated Valhalla levels to include.", "0,1,2"),
    ]
}

/// Basemap-only options, including the fork's landcover work.
pub fn basemap_options() -> Vec<OptionDef> {
    let mut defs = planetiler_common();
    defs.extend([
        opt("exclude_layers", "exclude_layers", "Exclude layers", "Layers",
            OptionKind::Text, "Comma-separated layers to leave out. The basemap excludes `route`.", "none"),
        opt("only_layers", "only_layers", "Only layers", "Layers",
            OptionKind::Text, "Comma-separated allow-list.", "all layers"),
        opt("transportation_name_limit_merge", "transportation-name-limit-merge", "Limit name merge", "Layers",
            OptionKind::Bool, "Restrict merging of transportation_name features.", "off"),
        opt("transportation_z13_paths", "transportation_z13_paths", "Paths at z13", "Layers",
            OptionKind::Bool, "Keep paths down to z13.", "off"),
        opt("landcover_tolerance_z11_13", "landcover_tolerance_z11_13", "Landcover tolerance z11-13", "Landcover",
            float(0.0),
            "Overrides landcover simplification for z11-13 only. Must exceed the global \
             simplify-tolerance to have any effect - a smaller value simplifies LESS and makes \
             the file bigger.",
            "the layer's own factor applies"),
        opt("landcover_drop_redundant_subclass", "landcover_drop_redundant_subclass", "Drop redundant subclass", "Landcover",
            OptionKind::Bool,
            "Omit `subclass` where it equals `class`. Small win - gzip already collapses the \
             repetition - and merging falls back to `class` so wood/grass still merge.",
            "off"),
        opt("landcover_merge_maxzoom", "landcover_merge_maxzoom", "Merge landcover at max zoom", "Landcover",
            OptionKind::Bool, "Extend polygon merging to z14.", "merging stops at z13"),
    ]);
    defs
}

/// Route-layer options from the fork.
pub fn routes_options() -> Vec<OptionDef> {
    let mut defs = planetiler_common();
    defs.extend([
        opt("only_layers", "only_layers", "Only layers", "Layers",
            OptionKind::Text, "Set to `route` for a routes-only build.", "all layers"),
        opt("route_road_tolerance", "route_road_tolerance", "Match road simplification", "Routes",
            OptionKind::Bool,
            "Simplify routes with the same tolerance the transportation layer uses \
             (`tolerance * 0.5`), so a route and the track it follows stay aligned at every zoom.",
            "routes use the plain tolerance and drift from roads"),
        opt("route_extent_digits", "route_extent_digits", "Extent decimals", "Routes",
            OptionKind::Int { min: Some(0), max: Some(9) },
            "Decimal places kept in the `extent` attribute. Extent is unique per relation, so \
             unlike duplicated attributes it does not compress away - trimming it is the single \
             biggest route-tile saving.",
            "3"),
        opt("route_symbol_id", "route_symbol_id", "Symbols as ids", "Routes",
            OptionKind::Bool,
            "Emit `osmc:symbol` as an integer id and write the lookup table alongside.",
            "the full symbol string is stored on every feature"),
        opt("route_symbol_table", "route_symbol_table", "Symbol table path", "Routes",
            OptionKind::Text, "Where the symbol id table is written.", "route_symbols.json"),
        opt("route_slim_attrs", "route_slim_attrs", "Slim attributes", "Routes",
            OptionKind::Bool, "Keep only osmid/class/network on tile features.", "off"),
        opt("route_drop_extent", "route_drop_extent", "Drop extent", "Routes",
            OptionKind::Bool, "Omit `extent` entirely.", "off"),
        opt("route_min_length", "route_min_length", "Drop short routes", "Routes",
            OptionKind::Bool, "Filter routes below a minimum rendered length.", "off"),
    ]);
    defs
}

/// Render a values map into planetiler arguments.
///
/// Only keys actually present are emitted, so an untouched form adds nothing to the command line
/// and planetiler's own defaults stand. Unknown keys are ignored rather than guessed at.
pub fn to_args(defs: &[OptionDef], values: &BTreeMap<String, Value>) -> Vec<String> {
    let mut args = Vec::new();
    for def in defs {
        let Some(value) = values.get(&def.key) else { continue };
        let rendered = match value {
            Value::Null => continue,
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => {
                // an empty string is meaningful for `languages` (drop all names), so it is
                // emitted rather than skipped
                s.clone()
            }
            other => other.to_string(),
        };
        args.push(format!("--{}={}", def.flag, rendered));
    }
    args
}

/// Look a definition up by key.
pub fn find<'a>(defs: &'a [OptionDef], key: &str) -> Option<&'a OptionDef> {
    defs.iter().find(|d| d.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn values(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn emits_only_what_was_set() {
        let defs = basemap_options();
        let args = to_args(&defs, &values(&[("simplify_tolerance", json!(0.7))]));
        assert_eq!(args, vec!["--simplify-tolerance=0.7"]);
    }

    /// An untouched form must add nothing, so planetiler's defaults stand rather than this
    /// schema's guesses about them.
    #[test]
    fn empty_values_emit_no_arguments() {
        assert!(to_args(&basemap_options(), &values(&[])).is_empty());
    }

    #[test]
    fn renders_the_measured_flag_set() {
        let defs = basemap_options();
        let args = to_args(
            &defs,
            &values(&[
                ("simplify_tolerance", json!(0.70)),
                ("simplify_tolerance_at_max_zoom", json!(0.25)),
                ("min_feature_size_at_max_zoom", json!(0.25)),
                ("landcover_tolerance_z11_13", json!(1.05)),
                ("landcover_drop_redundant_subclass", json!(true)),
                ("landcover_merge_maxzoom", json!(true)),
            ]),
        );
        assert!(args.contains(&"--simplify-tolerance=0.7".to_string()));
        assert!(args.contains(&"--simplify-tolerance-at-max-zoom=0.25".to_string()));
        assert!(args.contains(&"--landcover_tolerance_z11_13=1.05".to_string()));
        assert!(args.contains(&"--landcover_drop_redundant_subclass=true".to_string()));
        assert_eq!(args.len(), 6);
    }

    #[test]
    fn booleans_emit_explicit_false() {
        let args = to_args(&basemap_options(), &values(&[("compact_db", json!(false))]));
        assert_eq!(args, vec!["--compact-db=false"]);
    }

    /// `--languages=` with nothing after it is how localised names are dropped, so an empty
    /// string must survive rather than being treated as unset.
    #[test]
    fn empty_string_is_still_emitted() {
        let args = to_args(&basemap_options(), &values(&[("languages", json!(""))]));
        assert_eq!(args, vec!["--languages="]);
    }

    #[test]
    fn null_is_treated_as_unset() {
        assert!(to_args(&basemap_options(), &values(&[("simplify_tolerance", json!(null))])).is_empty());
    }

    #[test]
    fn unknown_keys_are_ignored() {
        assert!(to_args(&basemap_options(), &values(&[("not_a_flag", json!(1))])).is_empty());
    }

    #[test]
    fn route_options_carry_the_fork_flags() {
        let defs = routes_options();
        for key in ["route_road_tolerance", "route_extent_digits", "route_symbol_id"] {
            assert!(find(&defs, key).is_some(), "missing {key}");
        }
        let args = to_args(&defs, &values(&[("route_extent_digits", json!(2))]));
        assert_eq!(args, vec!["--route_extent_digits=2"]);
    }

    #[test]
    fn every_key_is_unique_within_a_step() {
        for defs in [basemap_options(), routes_options()] {
            let mut keys: Vec<&str> = defs.iter().map(|d| d.key.as_str()).collect();
            let before = keys.len();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), before, "duplicate option key");
        }
    }
}
