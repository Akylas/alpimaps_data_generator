//! Route against a Valhalla graph, optionally unpacked from a `.vtiles` package.
//!
//!   cargo run --release -p cairn-core --example route -- <tiles-dir-or-.vtiles> [costing]

use cairn_core::valhalla::{package, routing};

fn main() -> anyhow::Result<()> {
    if !routing::available() {
        println!("this build has no Valhalla linked; set VALHALLA_DIR and rebuild");
        return Ok(());
    }
    let mut args = std::env::args().skip(1);
    let input = std::path::PathBuf::from(
        args.next().expect("usage: route <valhalla.json> <tiles|.vtiles> [costing]"),
    );
    let tiles = std::path::PathBuf::from(args.next().expect("tiles path required"));
    let costing = args.next().unwrap_or_else(|| "auto".into());
    let (template, input) = (input, tiles);

    // routing against an unpacked package proves the artefact that actually ships, not just the
    // intermediate tile directory
    let tile_dir = if input.is_file() {
        let out = std::env::temp_dir().join("vtiles-unpacked");
        let _ = std::fs::remove_dir_all(&out);
        let started = std::time::Instant::now();
        let count = package::unpack(&input, &out)?;
        println!("unpacked {count} tiles from {} in {:.1}s", input.display(), started.elapsed().as_secs_f64());
        out
    } else {
        input
    };

    let started = std::time::Instant::now();
    let mut router = routing::Router::open(&template, &tile_dir)?;
    println!("actor ready in {:.2}s", started.elapsed().as_secs_f64());

    let request = routing::RouteRequest {
        // Grenoble to Chambery, both well inside the rhone-alpes extract
        locations: vec![[5.7245, 45.1885], [5.9178, 45.5646]],
        costing: costing.clone(),
    };
    let started = std::time::Instant::now();
    let response = router.route(&request.to_json())?;
    println!("routed with {costing} in {:.2}s", started.elapsed().as_secs_f64());

    let parsed: serde_json::Value = serde_json::from_str(&response)?;
    let leg = &parsed["trip"]["legs"][0];
    let summary = &parsed["trip"]["summary"];
    println!(
        "\ndistance {:.1} km, time {:.0} min",
        summary["length"].as_f64().unwrap_or(0.0),
        summary["time"].as_f64().unwrap_or(0.0) / 60.0
    );

    let maneuvers = leg["maneuvers"].as_array().cloned().unwrap_or_default();
    println!("{} maneuvers; first few:", maneuvers.len());
    for m in maneuvers.iter().take(6) {
        println!(
            "  {:>6.2} km  {}",
            m["length"].as_f64().unwrap_or(0.0),
            m["instruction"].as_str().unwrap_or("")
        );
    }
    let shape = leg["shape"].as_str().unwrap_or("");
    println!("\nencoded shape: {} chars", shape.len());
    Ok(())
}
