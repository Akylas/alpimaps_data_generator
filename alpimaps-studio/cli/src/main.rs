//! `alpimaps` - the pipeline from a terminal.
//!
//! Every subcommand runs the same code the desktop app runs, so a build started here and one
//! started from the GUI produce the same bytes. That is the point of the split: `studio-core`
//! holds the pipeline, this and the Tauri shell are two ways of asking it to run.
//!
//! Replaces the shell scripts it mirrors - `buildAll.sh`, `build_terrain_rgb.py`,
//! `build_valhalla_package.py` - and keeps their shape, so the flags read the same way.

mod steps;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "alpimaps",
    about = "Build, inspect and serve AlpiMaps tile output",
    version
)]
struct Cli {
    /// Repository root. Every other path defaults from it.
    #[arg(long, global = true, default_value = ".")]
    repo: PathBuf,

    /// Output root. Defaults to <repo>/alpimaps_mbtiles.
    #[arg(long, global = true)]
    output: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List generated areas and their artifacts.
    Catalog(steps::catalog::Args),
    /// Build the basemap vector tiles.
    Basemap(steps::planetiler::Args),
    /// Build the routes vector tiles.
    Routes(steps::planetiler::Args),
    /// Build terrain-RGB tiles from the sources in sources.json.
    Terrain(steps::terrain::Args),
    /// Pack a Valhalla tile directory into a .vtiles routing package.
    Package(steps::valhalla::PackageArgs),
    /// Expand a .vtiles package back into a tile directory.
    Unpack(steps::valhalla::UnpackArgs),
    /// Route between points using a package or tile directory.
    Route(steps::valhalla::RouteArgs),
    /// Sample an elevation profile along a line.
    Profile(steps::profile::Args),
    /// Serve the output over HTTP for a browser or the desktop app.
    Serve(steps::serve::Args),
    /// Show the options a build step accepts.
    Options(steps::options::Args),
    /// Show or clear what is already built for an area.
    State(steps::state::Args),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let settings = steps::settings_for(&cli.repo, cli.output.clone())?;

    match cli.command {
        Command::Catalog(args) => steps::catalog::run(&settings, args),
        Command::Basemap(args) => steps::planetiler::run(&settings, args, false).await,
        Command::Routes(args) => steps::planetiler::run(&settings, args, true).await,
        Command::Terrain(args) => steps::terrain::run(&settings, args),
        Command::Package(args) => steps::valhalla::package(&settings, args),
        Command::Unpack(args) => steps::valhalla::unpack(args),
        Command::Route(args) => steps::valhalla::route(&settings, args),
        Command::Profile(args) => steps::profile::run(args),
        Command::Serve(args) => steps::serve::run(&settings, args).await,
        Command::Options(args) => steps::options::run(args),
        Command::State(args) => steps::state::run(&settings, args),
    }
}
