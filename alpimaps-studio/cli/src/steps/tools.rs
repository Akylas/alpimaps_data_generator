//! `alpimaps download` / `elevation` / `valhalla-tiles` - the steps that were shell before.

use anyhow::{anyhow, Result};
use clap::Args as ClapArgs;
use std::collections::BTreeMap;
use studio_core::settings::Settings;
use studio_core::steps::external::{self, ToolJob};
use studio_core::steps::{download, state, StepEvent, StepId};

/// Mirrors `scripts/download-osm.py`: fetch one area's extract, wherever it comes from.
#[derive(ClapArgs)]
pub struct DownloadArgs {
    /// Geofabrik area id, e.g. `rhone-alpes`. Names the output when `--output` is not given.
    #[arg(long)]
    pub area: String,
    /// Download this URL instead of resolving the area through Geofabrik's index.
    #[arg(long)]
    pub url: Option<String>,
    /// Where to write the .pbf. Defaults to <repo>/data/sources/<area>.osm.pbf.
    #[arg(long)]
    pub output: Option<std::path::PathBuf>,
    /// Stop if the extract is already there.
    #[arg(long)]
    pub skip_existing: bool,
    /// Print the resolved URL and stop.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn download(settings: &Settings, args: DownloadArgs) -> Result<()> {
    let url = match &args.url {
        Some(url) => url.clone(),
        None => download::resolve_url(&args.area).await?,
    };
    if args.dry_run {
        println!("{url}");
        return Ok(());
    }
    let target = args
        .output
        .clone()
        .unwrap_or_else(|| download::extract_path(&settings.data_dir, &args.area));
    if args.skip_existing && target.is_file() {
        println!("{} is already there", target.display());
        return Ok(());
    }

    let mut last = u8::MAX;
    let path = download::fetch_url(&url, &target, |done, total| {
        let percent = match total {
            Some(total) if total > 0 => ((done * 100) / total).min(100) as u8,
            _ => 0,
        };
        if percent != last {
            last = percent;
            print!("\r  {percent:>3}%  {}", super::mb(done));
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
    })
    .await?;
    println!("\n  {}", path.display());
    // recording is a courtesy to the app's build view; it never gates what the CLI does
    let _ = state::mark_done(
        &settings.area_dir(&args.area),
        StepId::DownloadOsm,
        None,
        &BTreeMap::new(),
    );
    Ok(())
}

#[derive(ClapArgs)]
pub struct ToolArgs {
    /// Area the run is for. Names the build record, and finds the OSM extract.
    #[arg(long)]
    pub area: String,
    /// valhalla.json to use. Defaults to <repo>/valhalla.json.
    #[arg(long)]
    pub config: Option<std::path::PathBuf>,
    /// Directory holding the Valhalla binaries. Defaults to <repo>/valhalla/build, then PATH.
    #[arg(long)]
    pub bin_dir: Option<std::path::PathBuf>,
    /// `w,s,e,n` for valhalla_build_elevation, instead of taking bounds from the config.
    #[arg(long)]
    pub bbox: Option<String>,
    /// Elevation output directory, or the .osm.pbf for valhalla_build_tiles.
    #[arg(long)]
    pub input: Option<std::path::PathBuf>,
    /// Where valhalla_build_elevation writes. Defaults to <repo>/elevation_tiles.
    #[arg(long)]
    pub output: Option<std::path::PathBuf>,
    /// Stop if the output is already there.
    #[arg(long)]
    pub skip_existing: bool,
    /// Print the command that would run, and stop.
    #[arg(long)]
    pub dry_run: bool,
    /// Extra arguments passed to the binary verbatim, after `--`.
    #[arg(last = true)]
    pub passthrough: Vec<String>,
}

/// `valhalla_build_elevation` - the `.hgt` tiles the graph bakes in.
pub async fn elevation(settings: &Settings, args: ToolArgs) -> Result<()> {
    let config = args.config.clone().unwrap_or_else(|| settings.valhalla_config_path());
    let out = args.output.clone().unwrap_or_else(|| settings.elevation_tiles_dir.clone());
    // `-d` writes decompressed .hgt, which is what the terrain step reads later
    let mut tool_args = vec!["-v".to_string(), "-d".to_string()];
    match &args.bbox {
        // an explicit box beats the config: it is how you fetch a parent area's elevation
        Some(bbox) => tool_args.extend(["-b".to_string(), bbox.clone()]),
        None => tool_args.extend(["-c".to_string(), config.display().to_string()]),
    }
    tool_args.extend(["-o".to_string(), out.display().to_string()]);
    tool_args.extend(args.passthrough.clone());

    let job = ToolJob {
        step: StepId::ElevationTiles,
        area: args.area.clone(),
        program: bin(&args, settings, "valhalla_build_elevation")?,
        args: tool_args,
        working_dir: settings.repo_root.clone(),
    };
    run_tool(settings, args, job, out).await
}

fn bin(args: &ToolArgs, settings: &Settings, name: &str) -> Result<std::path::PathBuf> {
    let dir = args.bin_dir.clone().or_else(|| settings.valhalla_bin_dir.clone());
    external::find_tool(dir.as_deref(), name)
        .ok_or_else(|| anyhow!("{name} not found; build the Valhalla submodule or pass --bin-dir"))
}

/// `valhalla_build_tiles` - the routing graph itself.
pub async fn valhalla_tiles(settings: &Settings, args: ToolArgs) -> Result<()> {
    let config = args.config.clone().unwrap_or_else(|| settings.valhalla_config_path());
    let pbf = args
        .input
        .clone()
        .unwrap_or_else(|| download::extract_path(&settings.data_dir, &args.area));
    if !pbf.is_file() {
        return Err(anyhow!(
            "{} is missing; run `alpimaps download --area {}` or pass --input",
            pbf.display(),
            args.area
        ));
    }
    let mut tool_args =
        vec!["-c".to_string(), config.display().to_string(), pbf.display().to_string()];
    tool_args.extend(args.passthrough.clone());

    // the graph lands wherever the config's mjolnir.tile_dir points, which is the config's
    // business; the default is what `state` also looks at
    let out = args.output.clone().unwrap_or_else(|| settings.repo_root.join("valhalla_tiles"));
    let job = ToolJob {
        step: StepId::ValhallaTiles,
        area: args.area.clone(),
        program: bin(&args, settings, "valhalla_build_tiles")?,
        args: tool_args,
        working_dir: settings.repo_root.clone(),
    };
    run_tool(settings, args, job, out).await
}

async fn run_tool(
    settings: &Settings,
    args: ToolArgs,
    job: ToolJob,
    output: std::path::PathBuf,
) -> Result<()> {
    let step = job.step;
    if args.dry_run {
        println!("{}", job.command_line().join(" "));
        return Ok(());
    }
    let occupied = std::fs::read_dir(&output).map(|mut e| e.next().is_some()).unwrap_or(false);
    if args.skip_existing && occupied {
        println!("{} already holds something", output.display());
        return Ok(());
    }

    let started = std::time::Instant::now();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StepEvent>(512);
    let handle = tokio::spawn(external::run(job, tx, tokio::sync::mpsc::channel(1).1));

    while let Some(event) = rx.recv().await {
        match event {
            StepEvent::Phase { name, .. } => println!("[{name}]"),
            // these tools are slow and quiet; their own log is the only progress there is
            StepEvent::Log { line, .. } => println!("{line}"),
            _ => {}
        }
    }

    match handle.await? {
        Ok(true) => {
            let _ = state::mark_done(
                &settings.area_dir(&args.area),
                step,
                Some(super::planetiler::human_elapsed(started.elapsed())),
                &BTreeMap::new(),
            );
            Ok(())
        }
        Ok(false) => Err(anyhow!("{} exited non-zero", step.label())),
        Err(e) => Err(e),
    }
}
