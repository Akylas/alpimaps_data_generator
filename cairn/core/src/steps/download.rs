//! `download_osm` - fetch an area's OSM extract, the way planetiler would.
//!
//! Planetiler's `--download` does this on its own, but only as part of a build. As a step of its
//! own it can be run once and reused: the same `.osm.pbf` feeds the basemap, the routes and the
//! Valhalla graph, and downloading it three times over a slow link is the difference between a
//! morning and an afternoon.
//!
//! Extracts are resolved through Geofabrik's index rather than by guessing a URL, because the
//! path for an area is not derivable from its name (`rhone-alpes` lives under `europe/france/`).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;

/// The geometry-free index: same ids and URLs, 0.5 MB instead of 3.8 MB. The full one is slow
/// enough over a normal link to time the request out, and nothing here reads the polygons.
const INDEX_URL: &str = "https://download.geofabrik.de/index-v1-nogeom.json";
/// Kept as a fallback in case the trimmed index ever stops being published.
const INDEX_FALLBACK_URL: &str = "https://download.geofabrik.de/index-v1.json";

/// Where an area's extract is written. Planetiler spells its downloads with underscores, and
/// this has to match or it downloads its own copy.
pub fn extract_path(data_dir: &Path, area: &str) -> PathBuf {
    data_dir.join(format!("{}.osm.pbf", area.replace('-', "_")))
}

/// Find the extract URL for an area id, as Geofabrik names it.
pub async fn resolve_url(area: &str) -> Result<String> {
    let index = fetch_index().await?;

    let features = index["features"]
        .as_array()
        .ok_or_else(|| anyhow!("the Geofabrik index has no features"))?;

    let wanted = area.to_lowercase();
    let mut fallback = None;
    for feature in features {
        let props = &feature["properties"];
        let id = props["id"].as_str().unwrap_or_default();
        let name = props["name"].as_str().unwrap_or_default();
        let Some(pbf) = props["urls"]["pbf"].as_str() else { continue };
        if id.eq_ignore_ascii_case(&wanted) {
            return Ok(pbf.to_string());
        }
        // a name match is worth offering when the id does not match, but never over one
        if fallback.is_none() && name.to_lowercase() == wanted {
            fallback = Some(pbf.to_string());
        }
    }
    fallback.ok_or_else(|| {
        anyhow!("no Geofabrik extract called `{area}` - ids look like `rhone-alpes` or `france`")
    })
}

async fn fetch_index() -> Result<serde_json::Value> {
    let mut last: Option<anyhow::Error> = None;
    for url in [INDEX_URL, INDEX_FALLBACK_URL] {
        // the `json` feature is off, so read the document and parse it here
        match reqwest::get(url).await.and_then(|r| r.error_for_status()) {
            Ok(response) => match response.text().await {
                Ok(body) => {
                    return serde_json::from_str(&body).with_context(|| format!("parsing {url}"));
                }
                Err(e) => last = Some(anyhow::Error::new(e).context(format!("reading {url}"))),
            },
            Err(e) => last = Some(anyhow::Error::new(e).context(format!("fetching {url}"))),
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("could not fetch the Geofabrik index")))
}

/// Where an area's boundary polygon lives once fetched.
pub fn poly_path(data_dir: &Path, area: &str) -> PathBuf {
    data_dir.join(format!("{area}.poly"))
}

/// Geofabrik publishes the boundary beside the extract, but does not list it in the index: the
/// pbf is `.../rhone-alpes-latest.osm.pbf` and the polygon `.../rhone-alpes.poly`.
pub fn poly_url_from_pbf(pbf: &str) -> Option<String> {
    pbf.strip_suffix("-latest.osm.pbf").map(|base| format!("{base}.poly"))
}

/// The area's boundary polygon, downloading it if it is not already there.
///
/// Clipping to the area's own boundary is what stops a build writing half-filled tiles all around
/// the extract's bounding box, so it should not require hunting down a file by hand.
pub async fn ensure_poly<F>(data_dir: &Path, area: &str, progress: F) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>),
{
    let target = poly_path(data_dir, area);
    if target.is_file() {
        return Ok(target);
    }
    let pbf = resolve_url(area).await?;
    let url = poly_url_from_pbf(&pbf)
        .ok_or_else(|| anyhow!("cannot derive a .poly URL from `{pbf}`"))?;
    fetch_url(&url, &target, progress).await
}

/// Resolve the `area_poly` switch into a concrete clip path in `values`.
///
/// `clip_key` is whichever key that step uses for a shape - planetiler calls it `polygon`, the
/// terrain renderer `poly-shape`. An explicit shape always wins; `area_poly` only fills a gap.
/// The switch itself is removed either way, since it is cairn's own and means nothing downstream.
pub async fn apply_area_poly<F>(
    values: &mut std::collections::BTreeMap<String, serde_json::Value>,
    clip_key: &str,
    data_dir: &Path,
    area: &str,
    progress: F,
) -> Result<Option<PathBuf>>
where
    F: FnMut(u64, Option<u64>),
{
    let wanted = values.remove("area_poly").and_then(|v| v.as_bool()).unwrap_or(false);
    if !wanted || values.contains_key(clip_key) {
        return Ok(None);
    }
    let path = ensure_poly(data_dir, area, progress).await?;
    values.insert(clip_key.to_string(), serde_json::Value::String(path.display().to_string()));
    Ok(Some(path))
}

/// Download the extract for an area, reporting `(done, total)` bytes as it goes.
///
/// Writes through a `.part` file so an interrupted download cannot be mistaken for a finished
/// one - which matters here, because a step is considered built from the file being there.
pub async fn fetch<F>(data_dir: &Path, area: &str, progress: F) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>),
{
    let url = resolve_url(area).await?;
    fetch_url(&url, &extract_path(data_dir, area), progress).await
}

/// Download one URL to one path, reporting `(done, total)` bytes as it goes.
pub async fn fetch_url<F>(url: &str, target: &Path, mut progress: F) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>),
{
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = target.with_extension(match target.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.part"),
        None => "part".to_string(),
    });

    let response = reqwest::get(url).await.with_context(|| format!("downloading {url}"))?;
    if !response.status().is_success() {
        return Err(anyhow!("{url} returned {}", response.status()));
    }
    let total = response.content_length();

    let mut file = tokio::fs::File::create(&part).await?;
    let mut stream = response.bytes_stream();
    let mut done: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        done += chunk.len() as u64;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
        progress(done, total);
    }
    tokio::io::AsyncWriteExt::flush(&mut file).await?;
    drop(file);
    std::fs::rename(&part, target)?;
    Ok(target.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The index lists no polygon URL, so it is derived from the extract's. If Geofabrik ever
    /// renames the extract this returns None rather than inventing a URL that 404s.
    #[test]
    fn poly_url_is_derived_from_the_extract_url() {
        assert_eq!(
            poly_url_from_pbf("https://download.geofabrik.de/europe/france/rhone-alpes-latest.osm.pbf")
                .as_deref(),
            Some("https://download.geofabrik.de/europe/france/rhone-alpes.poly")
        );
        assert_eq!(poly_url_from_pbf("https://example.invalid/rhone-alpes.osm.pbf"), None);
    }

    /// An explicit clip shape is the user's decision and must survive the switch.
    #[tokio::test]
    async fn area_poly_never_overrides_an_explicit_shape() {
        let mut values = std::collections::BTreeMap::from([
            ("area_poly".to_string(), serde_json::json!(true)),
            ("polygon".to_string(), serde_json::json!("/tmp/mine.poly")),
        ]);
        let used = apply_area_poly(&mut values, "polygon", Path::new("/nope"), "rhone-alpes", |_, _| {})
            .await
            .expect("an explicit shape short-circuits before any download");
        assert_eq!(used, None);
        assert_eq!(values["polygon"], serde_json::json!("/tmp/mine.poly"));
        // the switch is cairn's own and must not reach the tool
        assert!(!values.contains_key("area_poly"));
    }

    /// The polygon keeps the area's own spelling; only the extract is renamed for planetiler.
    #[test]
    fn poly_path_keeps_the_area_name() {
        let p = poly_path(Path::new("/data"), "rhone-alpes");
        assert_eq!(p, Path::new("/data/rhone-alpes.poly"));
    }

    /// Planetiler looks for `rhone_alpes.osm.pbf`, not `rhone-alpes.osm.pbf`. Getting this wrong
    /// means it silently downloads a second copy of a 400 MB file.
    #[test]
    fn the_extract_is_named_the_way_planetiler_expects() {
        assert_eq!(
            extract_path(Path::new("/data"), "rhone-alpes"),
            PathBuf::from("/data/rhone_alpes.osm.pbf")
        );
    }
}
