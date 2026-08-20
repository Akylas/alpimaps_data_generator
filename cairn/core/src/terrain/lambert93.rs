//! Lambert-93 (EPSG:2154), the projection the IGN elevation raster is stored in.
//!
//! A Lambert Conformal Conic with two standard parallels on GRS80. Implemented directly rather
//! than by binding PROJ: the formulas are fully specified by EPSG 9802, and pulling in a C
//! library for one projection would undo the point of a pure-Rust pipeline.
//!
//! RGF93 and WGS84 differ by a few centimetres, which is far below the 5 m raster's resolution,
//! so the datum shift is deliberately ignored.

/// GRS80.
const A: f64 = 6_378_137.0;
const INV_F: f64 = 298.257_222_101;

/// EPSG:2154 parameters.
const LAT_ORIGIN: f64 = 46.5;
const LON_ORIGIN: f64 = 3.0;
const LAT_1: f64 = 49.0;
const LAT_2: f64 = 44.0;
const EASTING_ORIGIN: f64 = 700_000.0;
const NORTHING_ORIGIN: f64 = 6_600_000.0;

fn eccentricity() -> f64 {
    let f = 1.0 / INV_F;
    (2.0 * f - f * f).sqrt()
}

fn m_of(lat: f64, e: f64) -> f64 {
    lat.cos() / (1.0 - e * e * lat.sin() * lat.sin()).sqrt()
}

fn t_of(lat: f64, e: f64) -> f64 {
    let s = lat.sin();
    (std::f64::consts::FRAC_PI_4 - lat / 2.0).tan() / ((1.0 - e * s) / (1.0 + e * s)).powf(e / 2.0)
}

/// Constants derived once from the projection parameters.
struct Constants {
    e: f64,
    n: f64,
    af: f64,
    r0: f64,
    lon0: f64,
}

/// Computed once. These are six transcendentals, and the projection is called per output pixel -
/// at 512 squared per tile, recomputing them dominated the render.
fn constants() -> &'static Constants {
    static CACHE: std::sync::OnceLock<Constants> = std::sync::OnceLock::new();
    CACHE.get_or_init(build_constants)
}

fn build_constants() -> Constants {
    let e = eccentricity();
    let (p1, p2, p0) = (LAT_1.to_radians(), LAT_2.to_radians(), LAT_ORIGIN.to_radians());
    let (m1, m2) = (m_of(p1, e), m_of(p2, e));
    let (t1, t2, t0) = (t_of(p1, e), t_of(p2, e), t_of(p0, e));
    let n = (m1.ln() - m2.ln()) / (t1.ln() - t2.ln());
    let f = m1 / (n * t1.powf(n));
    let af = A * f;
    Constants { e, n, af, r0: af * t0.powf(n), lon0: LON_ORIGIN.to_radians() }
}

/// Projected metres to longitude and latitude in degrees.
pub fn to_lonlat(easting: f64, northing: f64) -> (f64, f64) {
    let c = constants();
    let dx = easting - EASTING_ORIGIN;
    let dy = c.r0 - (northing - NORTHING_ORIGIN);

    let r = (dx * dx + dy * dy).sqrt() * c.n.signum();
    let t = (r / c.af).powf(1.0 / c.n);
    let theta = dx.atan2(dy);
    let lon = theta / c.n + c.lon0;

    // latitude has no closed form; the standard fixed-point iteration converges in a handful of
    // rounds at this eccentricity
    let mut lat = std::f64::consts::FRAC_PI_2 - 2.0 * t.atan();
    for _ in 0..12 {
        let s = lat.sin();
        let next = std::f64::consts::FRAC_PI_2
            - 2.0 * (t * ((1.0 - c.e * s) / (1.0 + c.e * s)).powf(c.e / 2.0)).atan();
        if (next - lat).abs() < 1e-13 {
            lat = next;
            break;
        }
        lat = next;
    }
    (lon.to_degrees(), lat.to_degrees())
}

/// Longitude and latitude in degrees to projected metres.
pub fn from_lonlat(lon: f64, lat: f64) -> (f64, f64) {
    let c = constants();
    let t = t_of(lat.to_radians(), c.e);
    let r = c.af * t.powf(c.n);
    let theta = c.n * (lon.to_radians() - c.lon0);
    (
        EASTING_ORIGIN + r * theta.sin(),
        NORTHING_ORIGIN + c.r0 - r * theta.cos(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values produced with pyproj (EPSG:2154 -> EPSG:4326).
    const REFERENCE: [(f64, f64, f64, f64); 5] = [
        (700_000.0, 6_600_000.0, 3.000000000, 46.500000000),
        (94_997.5, 7_115_002.5, -5.588218833, 50.834984421),
        (900_000.0, 6_450_000.0, 5.543992255, 45.120282633),
        (1_000_000.0, 6_300_000.0, 6.724451741, 43.735337638),
        (650_000.0, 6_860_000.0, 2.318790597, 48.838110123),
    ];

    #[test]
    fn matches_pyproj() {
        for (x, y, want_lon, want_lat) in REFERENCE {
            let (lon, lat) = to_lonlat(x, y);
            assert!(
                (lon - want_lon).abs() < 1e-7 && (lat - want_lat).abs() < 1e-7,
                "({x}, {y}) gave ({lon:.9}, {lat:.9}), wanted ({want_lon:.9}, {want_lat:.9})"
            );
        }
    }

    /// The false origin is the one point whose answer is exact by definition.
    #[test]
    fn false_origin_is_exact() {
        let (lon, lat) = to_lonlat(EASTING_ORIGIN, NORTHING_ORIGIN);
        assert!((lon - LON_ORIGIN).abs() < 1e-12, "{lon}");
        assert!((lat - LAT_ORIGIN).abs() < 1e-9, "{lat}");
    }

    #[test]
    fn round_trips_within_a_millimetre() {
        for (x, y, _, _) in REFERENCE {
            let (lon, lat) = to_lonlat(x, y);
            let (bx, by) = from_lonlat(lon, lat);
            assert!((bx - x).abs() < 1e-3 && (by - y).abs() < 1e-3, "({x},{y}) -> ({bx},{by})");
        }
    }

    /// The corner of the actual raster, which is well outside France and therefore well outside
    /// the region the projection is tuned for - it still has to converge.
    #[test]
    fn converges_at_the_raster_corner() {
        let (lon, lat) = to_lonlat(94_997.5, 7_115_002.5);
        assert!(lon.is_finite() && lat.is_finite());
        assert!((-6.0..-5.0).contains(&lon), "{lon}");
    }
}
