//! `make_bathymap.py` as a build step.
//!
//! The only step here that is still a Python script, and it stays one for the same reason the
//! Valhalla tools stay C++ binaries: what it does is shapely and GEOS work - cumulative
//! dissolves, morphological generalisation, coverage-preserving simplification - and porting
//! that to Rust would be a rewrite of the interesting part, not a port of the boring part.
//!
//! What this module adds is what the app needs from it and a shell does not give: the argv it
//! will run, resolved interpreter and script paths with errors that say which one is missing,
//! and its `STAGE`/`PROGRESS` lines turned into events.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::archive::ArchiveFormat;
use super::options::{self, OptionDef};
use super::{StepEvent, StepId};

/// Everything one bathymap run needs.
pub struct BathymapJob {
    pub area: String,
    pub python: PathBuf,
    pub script: PathBuf,
    pub output: PathBuf,
    pub cache: PathBuf,
    /// `None` builds the whole world.
    pub bbox: Option<(f64, f64, f64, f64)>,
    pub versatiles: Option<PathBuf>,
    pub tippecanoe: Option<PathBuf>,
    pub values: BTreeMap<String, Value>,
    pub force: bool,
    pub working_dir: PathBuf,
}

impl BathymapJob {
    /// The argv, ready to run or to print.
    pub fn argv(&self) -> Vec<String> {
        let mut argv = vec![
            self.python.display().to_string(),
            // unbuffered, or the stage and progress lines arrive in one lump at the end and the
            // UI shows nothing for half an hour
            "-u".into(),
            self.script.display().to_string(),
        ];

        match self.bbox {
            Some((w, s, e, n)) => {
                argv.push("--bbox".into());
                argv.extend([w, s, e, n].iter().map(|v| format!("{v}")));
            }
            None => argv.push("--global".into()),
        }

        argv.push("--output".into());
        argv.push(self.output.display().to_string());
        argv.push("--cache".into());
        argv.push(self.cache.display().to_string());

        if let Some(path) = &self.versatiles {
            argv.push("--versatiles".into());
            argv.push(path.display().to_string());
        }
        if let Some(path) = &self.tippecanoe {
            argv.push("--tippecanoe".into());
            argv.push(path.display().to_string());
        }

        argv.extend(script_args(&bathymap_defs(), &self.values));

        if self.force {
            argv.push("--force".into());
        }
        argv
    }
}

fn bathymap_defs() -> Vec<OptionDef> {
    options::bathymap_options()
}

/// Render option values as the script's own `--flag value` pairs.
///
/// The script takes space-separated values, not planetiler's `--flag=value`, so this cannot go
/// through `options::to_args`. Booleans are switches: true emits the flag alone, false emits
/// nothing - `--global` and `--force` have no negative form.
pub fn script_args(defs: &[OptionDef], values: &BTreeMap<String, Value>) -> Vec<String> {
    let mut args = Vec::new();
    for def in defs {
        // handled by the job itself, not as a passthrough flag
        if def.flag.is_empty() {
            continue;
        }
        let Some(value) = values.get(&def.key) else { continue };
        match value {
            Value::Null => continue,
            Value::Bool(true) => args.push(format!("--{}", def.flag)),
            Value::Bool(false) => continue,
            Value::Number(n) => {
                args.push(format!("--{}", def.flag));
                args.push(n.to_string());
            }
            Value::String(s) if s.is_empty() => continue,
            Value::String(s) => {
                args.push(format!("--{}", def.flag));
                args.push(s.clone());
            }
            other => {
                args.push(format!("--{}", def.flag));
                args.push(other.to_string());
            }
        }
    }
    args
}

/// Find an interpreter that can actually run the script.
///
/// The repository's own venv first, because that is where geopandas and shapely are installed;
/// `python3` on PATH is the fallback for a checkout that never made one. A packaged build passes
/// its own path and never reaches either.
pub fn find_python(repo_root: &Path, configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = configured {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }
    let venv = repo_root.join("venv/bin/python3");
    if venv.is_file() {
        return Some(venv);
    }
    super::external::find_tool(std::iter::empty(), "python3")
}

/// Where the script lives, given a repository root.
pub fn script_path(repo_root: &Path) -> PathBuf {
    repo_root.join("scripts/make_bathymap.py")
}

/// Turn one of the script's log lines into an event.
///
/// Its own format, chosen for this: `[bathymap] STAGE <name> t=12.3s` and
/// `[bathymap] PROGRESS <label> 12/64 19%`. Anything else is an ordinary log line - the script
/// also relays versatiles and tippecanoe output verbatim, and none of that should be parsed.
pub fn parse_line(step: StepId, line: &str) -> StepEvent {
    // tippecanoe and versatiles draw their progress with carriage returns and no newline, so
    // the whole of it arrives here as one line - 200 kB of `12.5%  1/1/0` for a global build.
    // Only the last segment is current; the rest is a redraw history nobody wants in a log.
    let line = line.rsplit('\r').find(|s| !s.trim().is_empty()).unwrap_or(line);
    let rest = line.strip_prefix("[bathymap] ").unwrap_or(line);

    if let Some(stage) = rest.strip_prefix("STAGE ") {
        let name = stage.split_whitespace().next().unwrap_or(stage);
        return StepEvent::Phase { step, name: name.to_string() };
    }

    if let Some(progress) = rest.strip_prefix("PROGRESS ") {
        let mut parts = progress.split_whitespace();
        let label = parts.next().unwrap_or("").to_string();
        // "12/64" then "19%"
        let _counts = parts.next();
        if let Some(percent) = parts.next().and_then(|p| p.strip_suffix('%')) {
            if let Ok(percent) = percent.parse::<f64>() {
                return StepEvent::Progress {
                    step,
                    label,
                    percent: percent.round().clamp(0.0, 100.0) as u8,
                };
            }
        }
    }

    // tippecanoe's own tile progress, e.g. `  62.5%  5/16/11`. It is the only progress there is
    // during the last stage of a build, which on a global run is several minutes.
    if let Some(percent) = tippecanoe_percent(rest) {
        return StepEvent::Progress { step, label: "tiles".into(), percent };
    }

    StepEvent::Log { step, line: line.to_string() }
}

/// Read `  62.5%  5/16/11` as 62.
fn tippecanoe_percent(line: &str) -> Option<u8> {
    let mut parts = line.split_whitespace();
    let percent = parts.next()?.strip_suffix('%')?.parse::<f64>().ok()?;
    // a bare percentage is versatiles or a stray number; tippecanoe's carries the tile
    let tile = parts.next()?;
    let looks_like_a_tile =
        tile.split('/').count() == 3 && tile.split('/').all(|p| p.parse::<u32>().is_ok());
    looks_like_a_tile.then(|| percent.round().clamp(0.0, 100.0) as u8)
}

/// The output path for one area and format.
pub fn output_path(area_dir: &Path, area: &str, format: ArchiveFormat) -> PathBuf {
    area_dir.join(
        super::archive::archive_name(area, StepId::Bathymap, format)
            .expect("bathymap always has an archive name"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job() -> BathymapJob {
        BathymapJob {
            area: "world".into(),
            python: "/repo/venv/bin/python3".into(),
            script: "/repo/scripts/make_bathymap.py".into(),
            output: "/out/world_bathymap.mbtiles".into(),
            cache: "/repo/.cache/bathymap".into(),
            bbox: None,
            versatiles: None,
            tippecanoe: None,
            values: BTreeMap::new(),
            force: false,
            working_dir: "/repo".into(),
        }
    }

    #[test]
    fn a_global_run_passes_global_and_no_bbox() {
        let argv = job().argv();
        assert!(argv.contains(&"--global".to_string()));
        assert!(!argv.iter().any(|a| a == "--bbox"));
        assert_eq!(argv[1], "-u", "unbuffered, or progress arrives only at the end");
    }

    #[test]
    fn a_bbox_run_passes_four_separate_numbers() {
        let mut j = job();
        j.bbox = Some((-10.0, 30.0, 40.0, 48.0));
        let argv = j.argv();
        let at = argv.iter().position(|a| a == "--bbox").expect("bbox flag");
        assert_eq!(&argv[at + 1..at + 5], &["-10", "30", "40", "48"]);
        assert!(!argv.contains(&"--global".to_string()));
    }

    /// The script's flags take a separate value, unlike planetiler's `--flag=value`.
    #[test]
    fn options_render_as_separate_arguments() {
        let defs = bathymap_defs();
        let values: BTreeMap<String, Value> = [
            ("max_zoom".to_string(), json!(6)),
            ("landcover_smooth_pixels".to_string(), json!(12.0)),
        ]
        .into_iter()
        .collect();
        let args = script_args(&defs, &values);
        let at = args.iter().position(|a| a == "--max-zoom").expect("max-zoom");
        assert_eq!(args[at + 1], "6");
        let at = args.iter().position(|a| a == "--landcover-smooth-pixels").expect("smooth");
        assert_eq!(args[at + 1], "12.0");
    }

    /// A switch has no negative form, so false must emit nothing rather than `--force false`.
    #[test]
    fn false_switches_emit_nothing() {
        let defs = bathymap_defs();
        let values: BTreeMap<String, Value> =
            [("refresh_source".to_string(), json!(false))].into_iter().collect();
        assert!(script_args(&defs, &values).is_empty());
    }

    #[test]
    fn untouched_options_add_nothing() {
        assert!(script_args(&bathymap_defs(), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn stage_lines_become_phases() {
        match parse_line(StepId::Bathymap, "[bathymap] STAGE landcover t=141.4s") {
            StepEvent::Phase { name, .. } => assert_eq!(name, "landcover"),
            other => panic!("expected a phase, got {other:?}"),
        }
    }

    #[test]
    fn progress_lines_become_progress() {
        match parse_line(StepId::Bathymap, "[bathymap] PROGRESS landcover-z7 10/64 16%") {
            StepEvent::Progress { label, percent, .. } => {
                assert_eq!(label, "landcover-z7");
                assert_eq!(percent, 16);
            }
            other => panic!("expected progress, got {other:?}"),
        }
    }

    #[test]
    fn everything_else_stays_a_log_line() {
        for line in ["[bathymap] z7: 216991 polygons", "info: finished converting tiles"] {
            assert!(matches!(parse_line(StepId::Bathymap, line), StepEvent::Log { .. }), "{line}");
        }
    }

    /// tippecanoe redraws its progress with carriage returns and never emits a newline until it
    /// is done, so a whole build's worth of redraws reaches us as a single line. Keeping only
    /// the last segment is the difference between one progress reading and 200 kB of log.
    #[test]
    fn a_carriage_return_redraw_collapses_to_its_last_state() {
        let redraw = "  12.5%  1/1/0    \r  50.0%  4/8/5    \r  99.9%  7/71/47  ";
        match parse_line(StepId::Bathymap, redraw) {
            StepEvent::Progress { label, percent, .. } => {
                assert_eq!(label, "tiles");
                assert_eq!(percent, 100);
            }
            other => panic!("expected progress, got {other:?}"),
        }
    }

    #[test]
    fn tile_progress_becomes_progress() {
        match parse_line(StepId::Bathymap, "  62.5%  5/16/11") {
            StepEvent::Progress { percent, .. } => assert_eq!(percent, 63),
            other => panic!("expected progress, got {other:?}"),
        }
    }

    /// A percentage with no tile behind it is somebody else's output, not tippecanoe's.
    #[test]
    fn a_bare_percentage_is_not_tile_progress() {
        assert!(tippecanoe_percent("  50.0% done").is_none());
        assert!(tippecanoe_percent("Reordering geometry: 42%").is_none());
    }
}
