//! `alpimaps download` / `elevation` / `valhalla-tiles` - the steps that were shell before.

use anyhow::{anyhow, Result};
use clap::Args as ClapArgs;
use std::collections::BTreeMap;
use studio_core::settings::Settings;
use studio_core::steps::external::{self, ToolJob};
use studio_core::steps::{download, state, StepEvent, StepId};

#[derive(ClapArgs)]
pub struct DownloadArgs {
    /// Geofabrik area id, e.g. `rhone-alpes`.
    #[arg(long)]
    pub area: String,
    /// Download even if the extract is already there.
    #[arg(long)]
    pub force: bool,
    /// Print the resolved URL and stop.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn download(settings: &Settings, args: DownloadArgs) -> Result<()> {
    if args.dry_run {
        println!("{}", download::resolve_url(&args.area).await?);
        return Ok(());
    }
    if !args.force
        && state::status(settings, &args.area, StepId::DownloadOsm, &BTreeMap::new()).is_fresh()
    {
        println!(
            "{} is already there - pass --force to download it again",
            download::extract_path(&settings.data_dir, &args.area).display()
        );
        return Ok(());
    }

    let mut last = u8::MAX;
    let path = download::fetch(&settings.data_dir, &args.area, |done, total| {
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
    state::mark_done(&settings.area_dir(&args.area), StepId::DownloadOsm, None, &BTreeMap::new())?;
    Ok(())
}

#[derive(ClapArgs)]
pub struct ToolArgs {
    /// Area the run is for. Names the build record, and finds the OSM extract.
    #[arg(long)]
    pub area: String,
    /// valhalla.json to use. Defaults to the configured one.
    #[arg(long)]
    pub config: Option<std::path::PathBuf>,
    /// Run even if the output is already there.
    #[arg(long)]
    pub force: bool,
    /// Print the command that would run, and stop.
    #[arg(long)]
    pub dry_run: bool,
}

/// `valhalla_build_elevation` - the `.hgt` tiles the graph bakes in.
pub async fn elevation(settings: &Settings, args: ToolArgs) -> Result<()> {
    let config = args.config.clone().unwrap_or_else(|| settings.valhalla_config_path());
    let job = ToolJob {
        step: StepId::ElevationTiles,
        area: args.area.clone(),
        program: external::find_tool(
            settings.valhalla_bin_dir.as_deref(),
            "valhalla_build_elevation",
        )
        .ok_or_else(|| anyhow!("valhalla_build_elevation not found; build the submodule"))?,
        args: vec![
            "-v".into(),
            "-d".into(),
            "-c".into(),
            config.display().to_string(),
            "-o".into(),
            settings.elevation_tiles_dir.display().to_string(),
        ],
        working_dir: settings.repo_root.clone(),
    };
    run_tool(settings, args, job).await
}

/// `valhalla_build_tiles` - the routing graph itself.
pub async fn valhalla_tiles(settings: &Settings, args: ToolArgs) -> Result<()> {
    let config = args.config.clone().unwrap_or_else(|| settings.valhalla_config_path());
    let pbf = download::extract_path(&settings.data_dir, &args.area);
    if !pbf.is_file() {
        return Err(anyhow!("{} is missing; run `alpimaps download` first", pbf.display()));
    }
    let job = ToolJob {
        step: StepId::ValhallaTiles,
        area: args.area.clone(),
        program: external::find_tool(settings.valhalla_bin_dir.as_deref(), "valhalla_build_tiles")
            .ok_or_else(|| anyhow!("valhalla_build_tiles not found; build the submodule"))?,
        args: vec!["-c".into(), config.display().to_string(), pbf.display().to_string()],
        working_dir: settings.repo_root.clone(),
    };
    run_tool(settings, args, job).await
}

async fn run_tool(settings: &Settings, args: ToolArgs, job: ToolJob) -> Result<()> {
    let step = job.step;
    if args.dry_run {
        println!("{}", job.command_line().join(" "));
        return Ok(());
    }
    if !args.force && state::status(settings, &args.area, step, &BTreeMap::new()).is_fresh() {
        println!("{} is already built - pass --force to rebuild", step.label());
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
            state::mark_done(
                &settings.area_dir(&args.area),
                step,
                Some(super::planetiler::human_elapsed(started.elapsed())),
                &BTreeMap::new(),
            )?;
            Ok(())
        }
        Ok(false) => Err(anyhow!("{} exited non-zero", step.label())),
        Err(e) => Err(e),
    }
}
