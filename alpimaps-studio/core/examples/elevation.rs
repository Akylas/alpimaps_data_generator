//! Sample the real terrain archive at known points, and draw a profile.
//!
//!   cargo run -p studio-core --example elevation -- ../alpimaps_mbtiles/rhone-alpes/rhone-alpes_terrain.mbtiles

use studio_core::elevation::TerrainSampler;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: elevation <terrain.mbtiles>");
    let mut s = TerrainSampler::open(std::path::Path::new(&path))?;
    println!("encoding={:?} zooms {}-{}", s.encoding(), s.minzoom, s.maxzoom);

    // Summits, checked as the maximum within a ~1 km window rather than at a single point.
    // A point check measures the accuracy of the coordinate as much as the accuracy of the
    // data, and a knife-edge summit is exactly where a few hundred metres of horizontal error
    // costs a thousand metres of height.
    let known = [
        ("Mont Blanc", 6.86475, 45.83261, 4808.0),
        ("Barre des Ecrins", 6.35917, 44.92222, 4102.0),
        ("Grande Casse", 6.85140, 45.39860, 3855.0),
        ("Grenoble centre", 5.72400, 45.18800, 214.0),
        ("Lac du Bourget", 5.86000, 45.72000, 232.0),
    ];
    println!(
        "\n{:<20} {:>9} {:>9} {:>9} {:>8}",
        "point", "surveyed", "at point", "1km max", "delta"
    );
    for (name, lon, lat, truth) in known {
        let at_point = s.sample(lon, lat, s.maxzoom);
        // ~1 km box, stepped finely enough to land on the summit pixel
        let mut best: Option<f32> = None;
        for i in -20i32..=20 {
            for j in -20i32..=20 {
                let (dlon, dlat) = (i as f64 * 0.0005, j as f64 * 0.0005);
                if let Some(e) = s.sample(lon + dlon, lat + dlat, s.maxzoom) {
                    best = Some(best.map_or(e, |b: f32| b.max(e)));
                }
            }
        }
        match (at_point, best) {
            (Some(p), Some(b)) => println!(
                "{:<20} {:>9.0} {:>9.1} {:>9.1} {:>+8.1}",
                name, truth, p, b, b - truth as f32
            ),
            _ => println!("{name:<20} {truth:>9.0}       — no coverage"),
        }
    }

    // a traverse across the Vanoise, densified to 50 m
    let line = [[6.50, 45.35], [6.70, 45.40], [6.90, 45.42]];
    let p = s.profile(&line, s.maxzoom, 50.0, 3.0)?;
    println!(
        "\nprofile: {:.1} km, {} samples at z{}, {:.0}-{:.0} m, +{:.0} / -{:.0} m (threshold {} m), gaps {}",
        p.distance_m / 1000.0,
        p.points.len(),
        p.zoom,
        p.min_m.unwrap_or(0.0),
        p.max_m.unwrap_or(0.0),
        p.ascent_m,
        p.descent_m,
        p.threshold_m,
        p.gaps
    );

    // the same line with no hysteresis, to show what quantisation dither would have added
    let naive = s.profile(&line, s.maxzoom, 50.0, 0.0)?;
    println!(
        "  without hysteresis: +{:.0} / -{:.0} m  ({:+.0} m of phantom climb)",
        naive.ascent_m,
        naive.descent_m,
        naive.ascent_m - p.ascent_m
    );
    Ok(())
}
