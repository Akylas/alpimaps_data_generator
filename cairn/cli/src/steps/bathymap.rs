//! `cairn bathymap` - global landcover and sea-floor depth for the low zooms.

use anyhow::{anyhow, Result};
use clap::Args as ClapArgs;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

use cairn_core::settings::Settings;
use cairn_core::steps::archive::{self, ArchiveFormat, PMTILES_KEY};
use cairn_core::steps::bathymap::{self, BathymapJob};
use cairn_core::steps::external::{self, ToolJob};
use cairn_core::steps::{state, StepEvent, StepId};

#[derive(ClapArgs)]
pub struct Args {
    /// Area the run is recorded against, and whose bounds `--extent area` uses.
    #[arg(long)]
    pub area: String,
    /// `global` builds the world, `area` only the area's bounding box.
    #[arg(long, default_value = "global")]
    pub extent: String,
    /// west,south,east,north, instead of the area's own bounds.
    #[arg(long)]
    pub bbox: Option<String>,
    /// Where to write. Defaults to <output_root>/<area>/<area>_bathymap.<ext>.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Write PMTiles instead of MBTiles.
    #[arg(long)]
    pub pmtiles: bool,

    #[arg(long)]
    pub min_zoom: Option<u8>,
    #[arg(long)]
    pub max_zoom: Option<u8>,
    /// Landcover smoothing radius, in tile units at each zoom.
    #[arg(long)]
    pub landcover_smooth_pixels: Option<f64>,
    /// Landcover Douglas-Peucker tolerance, applied across all classes at once.
    #[arg(long)]
    pub landcover_simplify_pixels: Option<f64>,
    /// Smallest landcover zone kept, squared tile units.
    #[arg(long)]
    pub landcover_min_zone_pixels: Option<f64>,
    /// Depth Douglas-Peucker tolerance.
    #[arg(long)]
    pub bathymetry_simplify_pixels: Option<f64>,
    /// Smallest depth polygon kept, squared tile units.
    #[arg(long)]
    pub bathymetry_min_zone_pixels: Option<f64>,
    /// Tile coordinate resolution as a power of two.
    #[arg(long)]
    pub detail: Option<u8>,
    /// Zoom of the processing grid. 0 is one seamless chunk and no parallelism.
    #[arg(long)]
    pub chunk_zoom: Option<u8>,
    /// Worker processes for the landcover chunks.
    #[arg(long)]
    pub jobs: Option<u32>,
    /// Re-fetch the landcover extract rather than reusing the cached one.
    #[arg(long)]
    pub refresh_source: bool,

    /// Path to the python3 that has geopandas and shapely.
    #[arg(long)]
    pub python: Option<PathBuf>,
    /// Persistent cache directory. Defaults to <repo>/.cache/bathymap.
    #[arg(long)]
    pub cache: Option<PathBuf>,
    /// Rebuild derived products; downloads stay cached.
    #[arg(long)]
    pub force: bool,
    /// Stop if the archive is already there.
    #[arg(long)]
    pub skip_existing: bool,
    /// Print the command that would run, and stop.
    #[arg(long)]
    pub dry_run: bool,
}

impl Args {
    /// The option values, in the same shape the GUI collects them.
    ///
    /// One representation for both front ends: the flags here are clap's, the keys are the
    /// schema's, and everything downstream only ever sees the keys.
    fn values(&self) -> BTreeMap<String, Value> {
        let mut values = BTreeMap::new();
        let mut set = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                values.insert(key.to_string(), value);
            }
        };
        set("min_zoom", self.min_zoom.map(|v| json!(v)));
        set("max_zoom", self.max_zoom.map(|v| json!(v)));
        set("landcover_smooth_pixels", self.landcover_smooth_pixels.map(|v| json!(v)));
        set("landcover_simplify_pixels", self.landcover_simplify_pixels.map(|v| json!(v)));
        set("landcover_min_zone_pixels", self.landcover_min_zone_pixels.map(|v| json!(v)));
        set("bathymetry_simplify_pixels", self.bathymetry_simplify_pixels.map(|v| json!(v)));
        set("bathymetry_min_zone_pixels", self.bathymetry_min_zone_pixels.map(|v| json!(v)));
        set("detail", self.detail.map(|v| json!(v)));
        set("chunk_zoom", self.chunk_zoom.map(|v| json!(v)));
        set("jobs", self.jobs.map(|v| json!(v)));
        if self.refresh_source {
            values.insert("refresh_source".into(), json!(true));
        }
        if self.pmtiles {
            values.insert(PMTILES_KEY.into(), json!(true));
        }
        values
    }
}

fn parse_bbox(raw: &str) -> Result<(f64, f64, f64, f64)> {
    let parts: Vec<f64> = raw.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    if parts.len() != 4 {
        return Err(anyhow!("--bbox wants west,south,east,north"));
    }
    Ok((parts[0], parts[1], parts[2], parts[3]))
}

pub async fn run(settings: &Settings, args: Args) -> Result<()> {
    let step = StepId::Bathymap;
    let values = args.values();
    let format = ArchiveFormat::from_values(&values);

    let script = bathymap::script_path(&settings.repo_root);
    if !script.is_file() {
        return Err(anyhow!(
            "{} is missing - this step runs the script from the repository",
            script.display()
        ));
    }
    let python = bathymap::find_python(&settings.repo_root, args.python.as_deref())
        .ok_or_else(|| anyhow!("no python3 with geopandas - pass --python"))?;

    let bbox = match (&args.bbox, args.extent.as_str()) {
        (Some(raw), _) => Some(parse_bbox(raw)?),
        (None, "global") => None,
        (None, "area") => Some(area_bounds(settings, &args.area).ok_or_else(|| {
            anyhow!(
                "no bounds for {} - build its basemap first, or pass --bbox",
                args.area
            )
        })?),
        (None, other) => return Err(anyhow!("--extent wants `global` or `area`, not `{other}`")),
    };

    let area_dir = settings.area_dir(&args.area);
    let output = args.output.clone().unwrap_or_else(|| {
        area_dir.join(
            archive::archive_name(&args.area, step, format).expect("bathymap has an archive name"),
        )
    });

    let job = BathymapJob {
        area: args.area.clone(),
        python,
        script,
        output: output.clone(),
        cache: args.cache.clone().unwrap_or_else(|| settings.repo_root.join(".cache/bathymap")),
        bbox,
        versatiles: external::find_tool(std::iter::empty(), "versatiles"),
        tippecanoe: external::find_tool(
            [settings.repo_root.join("tippecanoe")].iter().map(|p| p.as_path()),
            "tippecanoe",
        ),
        values,
        force: args.force,
        working_dir: settings.repo_root.clone(),
    };

    let argv = job.argv();
    if args.dry_run {
        let quoted: Vec<String> =
            argv.iter().map(|a| cairn_core::steps::shell_quote(a)).collect();
        println!("{}", quoted.join(" "));
        return Ok(());
    }
    if args.skip_existing && output.is_file() {
        println!("{} is already there", output.display());
        return Ok(());
    }
    std::fs::create_dir_all(&area_dir)?;

    let started = std::time::Instant::now();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StepEvent>(512);
    let tool = ToolJob {
        step,
        area: args.area.clone(),
        program: PathBuf::from(&argv[0]),
        args: argv[1..].to_vec(),
        working_dir: job.working_dir.clone(),
        parse: bathymap::parse_line,
    };
    let handle = tokio::spawn(external::run(tool, tx, tokio::sync::mpsc::channel(1).1));

    let mut on_progress_line = false;
    while let Some(event) = rx.recv().await {
        match event {
            // the script already prefixes every line with `[bathymap]`, so echoing the phase
            // separately would print each stage twice
            StepEvent::Log { line, .. } => {
                if on_progress_line {
                    println!();
                    on_progress_line = false;
                }
                println!("{line}");
            }
            // redrawn in place, the way the tools it wraps do it
            StepEvent::Progress { label, percent, .. } => {
                use std::io::Write;
                print!("\r  {label} {percent:>3}%");
                let _ = std::io::stdout().flush();
                on_progress_line = true;
            }
            _ => {}
        }
    }
    if on_progress_line {
        println!();
    }

    match handle.await? {
        Ok(true) => {
            let _ = state::mark_done(
                &area_dir,
                step,
                Some(super::planetiler::human_elapsed(started.elapsed())),
                &args.values(),
            );
            if let Ok(meta) = std::fs::metadata(&output) {
                println!("  {} ({})", output.display(), super::mb(meta.len()));
            }
            Ok(())
        }
        Ok(false) => Err(anyhow!("bathymap exited non-zero")),
        Err(e) => Err(e),
    }
}

/// The area's bounds, read from its basemap the way the Valhalla steps read them.
fn area_bounds(settings: &Settings, area: &str) -> Option<(f64, f64, f64, f64)> {
    let bounds = cairn_core::catalog::discover(&settings.output_root)
        .ok()?
        .into_iter()
        .find(|a| a.name == area)?
        .artifacts
        .iter()
        .find(|a| a.kind == cairn_core::catalog::ArtifactKind::Basemap)?
        .bounds
        .clone()?;
    let parts: Vec<f64> = bounds.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    (parts.len() == 4).then(|| (parts[0], parts[1], parts[2], parts[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> Args {
        Args {
            area: "world".into(),
            extent: "global".into(),
            bbox: None,
            output: None,
            pmtiles: false,
            min_zoom: None,
            max_zoom: None,
            landcover_smooth_pixels: None,
            landcover_simplify_pixels: None,
            landcover_min_zone_pixels: None,
            bathymetry_simplify_pixels: None,
            bathymetry_min_zone_pixels: None,
            detail: None,
            chunk_zoom: None,
            jobs: None,
            refresh_source: false,
            python: None,
            cache: None,
            force: false,
            skip_existing: false,
            dry_run: false,
        }
    }

    /// An untouched command line must add nothing, so the script's own defaults stand rather
    /// than this file's guesses about them.
    #[test]
    fn nothing_set_means_no_values() {
        assert!(args().values().is_empty());
    }

    #[test]
    fn set_flags_reach_the_schema_keys() {
        let mut a = args();
        a.max_zoom = Some(6);
        a.pmtiles = true;
        let values = a.values();
        assert_eq!(values.get("max_zoom"), Some(&json!(6)));
        assert_eq!(ArchiveFormat::from_values(&values), ArchiveFormat::PMTiles);
    }

    #[test]
    fn bbox_wants_four_numbers() {
        assert!(parse_bbox("-10,30,40").is_err());
        assert_eq!(parse_bbox("-10,30,40,48").unwrap(), (-10.0, 30.0, 40.0, 48.0));
    }
}
