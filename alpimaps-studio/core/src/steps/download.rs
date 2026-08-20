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

/// Download the extract for an area, reporting `(done, total)` bytes as it goes.
///
/// Writes through a `.part` file so an interrupted download cannot be mistaken for a finished
/// one - which matters here, because a step is considered built from the file being there.
pub async fn fetch<F>(data_dir: &Path, area: &str, mut progress: F) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>),
{
    let url = resolve_url(area).await?;
    std::fs::create_dir_all(data_dir)?;
    let target = extract_path(data_dir, area);
    let part = target.with_extension("pbf.part");

    let response = reqwest::get(&url).await.with_context(|| format!("downloading {url}"))?;
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
    std::fs::rename(&part, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

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
