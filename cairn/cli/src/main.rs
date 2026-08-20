//! `cairn` - the pipeline from a terminal.
//!
//! Every subcommand runs the same code the desktop app runs, so a build started here and one
//! started from the GUI produce the same bytes. That is the point of the split: `cairn-core`
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
    name = "cairn",
    about = "Build, inspect and serve AlpiMaps tile output",
    version
)]
pub struct Cli {
    /// Repository root. Every other path defaults from it.
    #[arg(long, global = true, default_value = ".")]
    pub repo: PathBuf,

    // Named `output_root`, not `output`: the field name is the clap id, and a global shares its
    // id with any subcommand arg of the same name. With a global `--output`, `terrain --output
    // file.mbtiles` set the output *root* and every later write looked for a directory inside an
    // mbtiles file.
    /// Output root. Defaults to <repo>/alpimaps_mbtiles. Per-step `--output` names one file.
    #[arg(long, global = true)]
    pub output_root: Option<PathBuf>,

    /// Where OSM extracts live. Defaults to <repo>/data/sources.
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,


    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List generated areas and their artifacts.
    Catalog(steps::catalog::Args),
    /// Download the area's OSM extract from Geofabrik.
    Download(steps::tools::DownloadArgs),
    /// Download the elevation tiles the Valhalla graph bakes in.
    Elevation(steps::tools::ToolArgs),
    /// Build the basemap vector tiles.
    Basemap(steps::planetiler::Args),
    /// Build the routes vector tiles.
    Routes(steps::planetiler::Args),
    /// Build terrain-RGB tiles from the sources in sources.json.
    Terrain(steps::terrain::Args),
    /// Build terrain-RGB tiles with the older mapbox packing, named _hillshade.
    Hillshade(steps::terrain::Args),
    /// Build the Valhalla routing graph from the OSM extract.
    ValhallaTiles(steps::tools::ToolArgs),
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
    let settings = steps::settings_for(&cli)?;

    match cli.command {
        Command::Catalog(args) => steps::catalog::run(&settings, args),
        Command::Download(args) => steps::tools::download(&settings, args).await,
        Command::Elevation(args) => steps::tools::elevation(&settings, args).await,
        Command::ValhallaTiles(args) => steps::tools::valhalla_tiles(&settings, args).await,
        Command::Basemap(args) => steps::planetiler::run(&settings, args, false).await,
        Command::Routes(args) => steps::planetiler::run(&settings, args, true).await,
        Command::Terrain(args) => steps::terrain::run(&settings, args, false).await,
        Command::Hillshade(args) => steps::terrain::run(&settings, args, true).await,
        Command::Package(args) => steps::valhalla::package(&settings, args),
        Command::Unpack(args) => steps::valhalla::unpack(args),
        Command::Route(args) => steps::valhalla::route(&settings, args),
        Command::Profile(args) => steps::profile::run(args),
        Command::Serve(args) => steps::serve::run(&settings, args).await,
        Command::Options(args) => steps::options::run(args),
        Command::State(args) => steps::state::run(&settings, args),
    }
}
