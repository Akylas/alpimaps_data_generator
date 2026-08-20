//! Read the IGN elevation raster with the pure-Rust BigTIFF reader and check it against GDAL.
//!
//!   cargo run --release -p cairn-core --example geotiff -- <path-to.tif>

use cairn_core::terrain::{geotiff::GeoTiff, lambert93};

/// Values from `gdallocationinfo -valonly -l_srs EPSG:4326`.
const REFERENCE: [(&str, f64, f64, f64); 5] = [
    ("Mont Blanc", 6.86475, 45.83261, 4767.22021484375),
    ("Grenoble", 5.724, 45.188, 211.880004882812),
    ("Lac du Bourget", 5.86, 45.72, 231.600006103516),
    ("Barre des Ecrins", 6.35917, 44.92222, 4068.86010742188),
    ("Lyon", 4.835, 45.764, 168.059997558594),
];

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: geotiff <file.tif>");
    let started = std::time::Instant::now();
    let mut tiff = GeoTiff::open(std::path::Path::new(&path))?;
    println!("opened in {:.2}s", started.elapsed().as_secs_f64());
    println!("nodata: {:?}", tiff.nodata);
    println!("\n{:<7} {:>10} {:>10} {:>9} {:>7} {:>6}", "level", "width", "height", "m/px", "tiles", "pred");
    for (i, l) in tiff.levels.iter().enumerate() {
        println!(
            "{:<7} {:>10} {:>10} {:>9.1} {:>7} {:>6}",
            i, l.width, l.height, l.scale.0, l.tile_count(), l.predictor
        );
    }
    let (w, s, e, n) = tiff.bounds();
    println!("\nprojected bounds: {w:.0}, {s:.0}, {e:.0}, {n:.0}");
    let (lon0, lat0) = lambert93::to_lonlat(w, n);
    let (lon1, lat1) = lambert93::to_lonlat(e, s);
    println!("geographic:       {lon0:.4}, {lat1:.4}, {lon1:.4}, {lat0:.4}");

    println!("\n{:<18} {:>12} {:>12} {:>9}", "point", "gdal", "rust", "delta");
    let mut worst = 0.0f64;
    for (name, lon, lat, want) in REFERENCE {
        let (easting, northing) = lambert93::from_lonlat(lon, lat);
        match tiff.sample(easting, northing, 0) {
            Some(got) => {
                let delta = got as f64 - want;
                worst = worst.max(delta.abs());
                println!("{name:<18} {want:>12.2} {:>12.2} {delta:>+9.3}", got);
            }
            None => println!("{name:<18} {want:>12.2} {:>12} ", "no data"),
        }
    }
    println!("\nworst delta {worst:.3} m");

    // overview levels should agree roughly, since they are downsampled from the same data
    println!("\nMont Blanc through the overview pyramid:");
    let (easting, northing) = lambert93::from_lonlat(6.86475, 45.83261);
    for level in 0..tiff.levels.len() {
        let m_per_px = tiff.levels[level].scale.0;
        match tiff.sample(easting, northing, level) {
            Some(v) => println!("  level {level} ({m_per_px:>6.0} m/px): {v:8.1} m"),
            None => println!("  level {level}: no data"),
        }
    }
    Ok(())
}
