//! `alpimaps serve` - the tile server, for a browser or the desktop app in dev mode.

use anyhow::Result;
use clap::Args as ClapArgs;
use std::path::PathBuf;
use std::sync::Arc;
use studio_core::settings::Settings;
use studio_core::valhalla::{package, routing};
use studio_core::{catalog, tileserver};

#[derive(ClapArgs)]
pub struct Args {
    #[arg(long, default_value_t = 8787)]
    pub port: u16,
    /// Also serve /route, from a .vtiles package or a tile directory.
    #[arg(long)]
    pub tiles: Option<PathBuf>,
    /// valhalla.json for the routing config. Defaults to <repo>/valhalla.json.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

pub async fn run(settings: &Settings, args: Args) -> Result<()> {
    let registry = Arc::new(tileserver::Registry::default());
    let areas = catalog::discover(&settings.output_root)?;
    let mut count = 0;
    for area in &areas {
        for art in &area.artifacts {
            // routing packages hold graph tiles, not map tiles
            if art.probe_error.is_some() || art.format == catalog::TileFormat::Gph3 {
                continue;
            }
            registry.set(
                format!("{}/{}", area.name, art.file_name),
                tileserver::Source::from_artifact(art),
            );
            count += 1;
        }
    }

    let router = match args.tiles {
        Some(tiles) if routing::available() => {
            let dir = if tiles.is_file() {
                let out = std::env::temp_dir().join("alpimaps-serve-tiles");
                if !out.is_dir() {
                    let n = package::unpack(&tiles, &out)?;
                    println!("unpacked {n} tiles for routing");
                }
                out
            } else {
                tiles
            };
            let template = args.config.unwrap_or_else(|| settings.repo_root.join("valhalla.json"));
            match routing::Router::open(&template, &dir) {
                Ok(router) => Some(router),
                Err(e) => {
                    println!("routing unavailable: {e}");
                    None
                }
            }
        }
        Some(_) => {
            println!("routing unavailable: this build has no Valhalla linked");
            None
        }
        None => None,
    };

    let handle =
        tileserver::start_full(args.port, registry, Some(settings.output_root.clone()), router).await?;
    let base = handle.base_url();
    println!("serving {count} sources on {base}");
    println!("  {base}/catalog");
    println!("  {base}/tilejson/<area>/<file>");
    println!("  {base}/tiles/<area>/<file>/{{z}}/{{x}}/{{y}}");
    println!("\nctrl-c to stop");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
