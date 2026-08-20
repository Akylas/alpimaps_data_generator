//! Start the tile server over a real output root and verify what it hands back.
//!
//!   cargo run -p cairn-core --example serve -- ../alpimaps_mbtiles [--hold]

use std::sync::Arc;
use cairn_core::valhalla::{package, routing};
use cairn_core::{catalog, tileserver};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("usage: serve <output_root> [--hold]"));
    let hold = args.any(|a| a == "--hold");

    let registry = Arc::new(tileserver::Registry::default());
    let areas = catalog::discover(&root)?;
    let mut probes = Vec::new();
    for area in &areas {
        for art in &area.artifacts {
            if art.probe_error.is_some() || art.format == catalog::TileFormat::Gph3 {
                continue;
            }
            let key = format!("{}/{}", area.name, art.file_name);
            // a zoom the archive actually covers, and a tile near the middle of its bounds
            let z = art.minzoom.unwrap_or(0).max(8).min(art.maxzoom.unwrap_or(14));
            probes.push((key.clone(), z, art.bounds.clone()));
            registry.set(key, tileserver::Source::from_artifact(art));
        }
    }

    // routing is opt-in: VALHALLA_CONFIG points at a valhalla.json, VALHALLA_TILES at either a
    // tile directory or a .vtiles package to unpack
    let router = match (std::env::var("VALHALLA_CONFIG"), std::env::var("VALHALLA_TILES")) {
        (Ok(config), Ok(tiles)) => {
            let tiles = std::path::PathBuf::from(tiles);
            let dir = if tiles.is_file() {
                let out = std::env::temp_dir().join("serve-vtiles");
                if !out.is_dir() {
                    let n = package::unpack(&tiles, &out)?;
                    println!("unpacked {n} tiles for routing");
                }
                out
            } else {
                tiles
            };
            match routing::Router::open(std::path::Path::new(&config), &dir) {
                Ok(r) => {
                    println!("routing ready on {}", dir.display());
                    Some(r)
                }
                Err(e) => {
                    println!("routing unavailable: {e}");
                    None
                }
            }
        }
        _ => None,
    };

    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8787);
    let server = tileserver::start_full(port, registry, Some(root.clone()), router).await?;
    let base = server.base_url();
    println!("serving {} sources on {base}\n", probes.len());

    let client = reqwest::Client::new();
    for (key, z, bounds) in probes {
        // centre of the declared bounds, converted to a tile at zoom z
        let (x, y) = match bounds.as_deref().and_then(|b| {
            let p: Vec<f64> = b.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            (p.len() == 4).then(|| ((p[0] + p[2]) / 2.0, (p[1] + p[3]) / 2.0))
        }) {
            Some((lon, lat)) => {
                let n = (1u32 << z) as f64;
                let x = ((lon + 180.0) / 360.0 * n) as u32;
                let lat_rad = lat.to_radians();
                let y = ((1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI)
                    / 2.0
                    * n) as u32;
                (x, y)
            }
            None => (0, 0),
        };

        let res = client
            .get(format!("{base}/tiles/{key}/{z}/{x}/{y}"))
            .header("accept-encoding", "identity")
            .send()
            .await?;
        let status = res.status();
        let ctype = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("-").to_string();
        let cenc = res.headers().get("content-encoding").and_then(|v| v.to_str().ok()).unwrap_or("-").to_string();
        let len = res.bytes().await?.len();
        println!("{key:<38} z{z}/{x}/{y}  {status}  {ctype:<42} enc={cenc:<5} {len} B");
    }

    if hold {
        println!("\nholding; ctrl-c to stop");
        tokio::signal::ctrl_c().await?;
    }
    Ok(())
}
