//! Local HTTP tile server, so MapLibre can read the generated archives directly.
//!
//! Three details decide whether a tile renders or silently fails:
//!
//! * **Row order.** mbtiles stores `tile_row` in TMS order (origin bottom-left); XYZ URLs count
//!   from the top. `tms_row = (1 << z) - 1 - y`.
//! * **Content-Encoding.** Planetiler writes gzipped blobs for MVT *and* MLT, but the terrain
//!   WebP tiles are stored raw. The `compression` metadata key is not reliable across producers,
//!   so the gzip magic (`1f 8b`) is sniffed per blob instead. Getting this wrong yields a
//!   silently blank map rather than an error.
//! * **Content-Type.** MLT needs `application/vnd.maplibre-vector-tile`, not the MVT type.
//!
//! Connections are pooled one-per-source behind a mutex. A tile read is an index lookup plus a
//! blob fetch - tens of microseconds - so serialising them costs less than the complexity of a
//! real pool, and the whole thing serves one local viewer.

use crate::catalog::{Artifact, TileFormat};
use anyhow::{Context, Result};
use axum::{
    extract::{Path as AxumPath, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;

/// One archive exposed by the server, keyed `<area>/<file_name>`.
pub struct Source {
    pub path: PathBuf,
    pub format: TileFormat,
    pub minzoom: u8,
    pub maxzoom: u8,
    pub bounds: Option<Vec<f64>>,
    pub encoding: Option<String>,
    pub tile_size: u32,
    pub vector_layers: Option<serde_json::Value>,
    conn: Mutex<Option<Connection>>,
}

impl Source {
    pub fn from_artifact(art: &Artifact) -> Self {
        let bounds = art.bounds.as_ref().and_then(|b| {
            let parts: Vec<f64> = b.split(',').filter_map(|p| p.trim().parse().ok()).collect();
            (parts.len() == 4).then_some(parts)
        });
        let vector_layers = art
            .metadata
            .get("json")
            .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
            .and_then(|v| v.get("vector_layers").cloned());
        Self {
            path: art.path.clone(),
            format: art.format.clone(),
            minzoom: art.minzoom.unwrap_or(0),
            maxzoom: art.maxzoom.unwrap_or(14),
            bounds,
            encoding: art.encoding.clone(),
            // terrain RGB is rendered at 512px; vector tiles use the 512 convention too
            tile_size: if art.format.is_vector() { 512 } else { 512 },
            vector_layers,
            conn: Mutex::new(None),
        }
    }

    fn content_type(&self) -> &'static str {
        match self.format {
            TileFormat::Mvt => "application/x-protobuf",
            TileFormat::Mlt => "application/vnd.maplibre-vector-tile",
            TileFormat::Webp => "image/webp",
            TileFormat::Png => "image/png",
            TileFormat::Jpeg => "image/jpeg",
            _ => "application/octet-stream",
        }
    }

    /// Fetch one XYZ tile, flipping to the TMS row mbtiles actually stores.
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>> {
        let tms_row = ((1u32 << z).checked_sub(1)).and_then(|max| max.checked_sub(y));
        let Some(tms_row) = tms_row else { return Ok(None) };

        let mut guard = self.conn.lock().map_err(|_| anyhow::anyhow!("source lock poisoned"))?;
        if guard.is_none() {
            *guard = Some(
                Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .with_context(|| format!("opening {}", self.path.display()))?,
            );
        }
        let conn = guard.as_ref().expect("just opened");
        let mut stmt = conn.prepare_cached(
            "SELECT tile_data FROM tiles WHERE zoom_level=? AND tile_column=? AND tile_row=?",
        )?;
        let blob: Option<Vec<u8>> = stmt
            .query_row((z, x, tms_row), |r| r.get(0))
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(blob)
    }
}

/// Is this blob gzipped? Sniffed rather than trusted from metadata, because producers disagree.
pub fn is_gzip(blob: &[u8]) -> bool {
    blob.len() >= 2 && blob[0] == 0x1f && blob[1] == 0x8b
}

#[derive(Default)]
pub struct Registry {
    sources: std::sync::RwLock<HashMap<String, Arc<Source>>>,
}

impl Registry {
    pub fn set(&self, key: String, source: Source) {
        if let Ok(mut map) = self.sources.write() {
            map.insert(key, Arc::new(source));
        }
    }

    pub fn get(&self, key: &str) -> Option<Arc<Source>> {
        self.sources.read().ok()?.get(key).cloned()
    }

    pub fn keys(&self) -> Vec<String> {
        self.sources.read().map(|m| m.keys().cloned().collect()).unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut map) = self.sources.write() {
            map.clear();
        }
    }
}

#[derive(Serialize)]
struct TileJson {
    tilejson: &'static str,
    tiles: Vec<String>,
    minzoom: u8,
    maxzoom: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    bounds: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vector_layers: Option<serde_json::Value>,
    /// Elevation packing for raster-DEM sources (`terrarium` / `mapbox`). MapLibre needs this
    /// verbatim on the style's source to decode heights.
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<String>,
    /// Tile container: `mvt` or `mlt`. Distinct from `encoding` above, which is about elevation
    /// packing - both are called "encoding" by MapLibre but on different source types, and
    /// conflating them silently produces either a flat hillshade or an undecodable vector tile.
    #[serde(rename = "tileEncoding")]
    tile_encoding: &'static str,
    #[serde(rename = "tileSize")]
    tile_size: u32,
}

#[derive(Clone)]
struct ServerState {
    registry: Arc<Registry>,
    base: String,
    /// Output root, so the server can answer catalog queries too. This is what lets the whole
    /// UI run against a plain browser instead of only inside the Tauri webview.
    output_root: Option<PathBuf>,
    /// A routing actor, when the server was started with one. Behind a mutex because `actor_t`
    /// is single-threaded and its graph reader caches between requests.
    router: Option<std::sync::Arc<Mutex<crate::valhalla::routing::Router>>>,
}

async fn tile_handler(
    State(state): State<ServerState>,
    AxumPath((area, file, z, x, y)): AxumPath<(String, String, u8, u32, String)>,
) -> Response {
    // the URL may carry an extension (`3.pbf`); MapLibre templates often include one
    let y = y.split('.').next().unwrap_or(&y);
    let Ok(y) = y.parse::<u32>() else {
        return (StatusCode::BAD_REQUEST, "bad y").into_response();
    };
    let Some(source) = state.registry.get(&format!("{area}/{file}")) else {
        return (StatusCode::NOT_FOUND, "unknown source").into_response();
    };

    match source.tile(z, x, y) {
        // an absent tile is normal for sparse coverage - 204 keeps it out of the error console
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Ok(Some(blob)) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(source.content_type()),
            );
            if is_gzip(&blob) {
                headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            }
            (headers, blob).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn tilejson_handler(
    State(state): State<ServerState>,
    AxumPath((area, file)): AxumPath<(String, String)>,
) -> Response {
    let Some(source) = state.registry.get(&format!("{area}/{file}")) else {
        return (StatusCode::NOT_FOUND, "unknown source").into_response();
    };
    Json(TileJson {
        tilejson: "3.0.0",
        tiles: vec![format!("{}/tiles/{area}/{file}/{{z}}/{{x}}/{{y}}", state.base)],
        minzoom: source.minzoom,
        maxzoom: source.maxzoom,
        bounds: source.bounds.clone(),
        vector_layers: source.vector_layers.clone(),
        encoding: source.encoding.clone(),
        tile_encoding: if source.format == TileFormat::Mlt { "mlt" } else { "mvt" },
        tile_size: source.tile_size,
    })
    .into_response()
}

async fn sources_handler(State(state): State<ServerState>) -> Json<Vec<String>> {
    Json(state.registry.keys())
}

async fn catalog_handler(State(state): State<ServerState>) -> Response {
    let Some(root) = state.output_root.clone() else {
        return (StatusCode::NOT_FOUND, "no output root configured").into_response();
    };
    match crate::catalog::discover(&root) {
        Ok(areas) => Json(areas).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// What is already built for an area, from the files in its output directory.
async fn build_state_handler(
    State(state): State<ServerState>,
    axum::extract::Path(area): axum::extract::Path<String>,
) -> Response {
    let Some(root) = state.output_root.clone() else {
        return (StatusCode::NOT_FOUND, "no output root configured").into_response();
    };
    // dev only: the browser has no settings store, so derive one from the root being served
    let mut settings =
        crate::settings::Settings::for_repo(root.parent().unwrap_or(&root).to_path_buf());
    settings.output_root = root.clone();
    let statuses =
        crate::steps::state::statuses(&settings, &area, &std::collections::BTreeMap::new());
    Json(statuses).into_response()
}

/// Step metadata, mirroring what the Tauri command returns so the UI can be driven from a
/// plain browser without a second definition of the step graph.
async fn steps_handler() -> Json<serde_json::Value> {
    let steps: Vec<serde_json::Value> = crate::steps::ALL_STEPS
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s,
                "label": s.label(),
                "deps": s.deps(),
                "implemented": s.is_implemented(),
            })
        })
        .collect();
    Json(serde_json::Value::Array(steps))
}

async fn step_options_handler(
    AxumPath(step): AxumPath<String>,
) -> Response {
    use crate::steps::options;
    let defs = match step.as_str() {
        "routes" => options::routes_options(),
        "basemap" => options::basemap_options(),
        _ => options::planetiler_common(),
    };
    Json(defs).into_response()
}

async fn presets_handler() -> Json<Vec<crate::presets::Preset>> {
    Json(crate::presets::builtin())
}

async fn plan_handler(Json(steps): Json<Vec<crate::steps::StepId>>) -> Json<Vec<crate::steps::StepId>> {
    Json(crate::steps::plan(&steps))
}

#[derive(serde::Deserialize)]
struct RouteBody {
    locations: Vec<[f64; 2]>,
    #[serde(default = "default_costing")]
    costing: String,
}

fn default_costing() -> String {
    "pedestrian".into()
}

async fn route_handler(State(state): State<ServerState>, Json(body): Json<RouteBody>) -> Response {
    let Some(router) = state.router.clone() else {
        return (StatusCode::NOT_FOUND, "server started without routing").into_response();
    };
    let request = crate::valhalla::routing::RouteRequest {
        locations: body.locations,
        costing: body.costing,
    }
    .to_json();
    let result = tokio::task::spawn_blocking(move || {
        let mut guard = router.lock().map_err(|_| anyhow::anyhow!("router lock poisoned"))?;
        guard.route(&request)
    })
    .await;
    match result {
        Ok(Ok(json)) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn routing_status_handler(State(state): State<ServerState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "available": crate::valhalla::routing::available() && state.router.is_some(),
    }))
}

#[derive(serde::Deserialize)]
struct ProfileBody {
    path: String,
    line: Vec<[f64; 2]>,
    #[serde(default)]
    zoom: Option<u8>,
    #[serde(default)]
    densify_m: Option<f64>,
    #[serde(default)]
    threshold_m: Option<f64>,
}

async fn profile_handler(Json(body): Json<ProfileBody>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let mut sampler = crate::elevation::TerrainSampler::open(std::path::Path::new(&body.path))?;
        let zoom = body.zoom.unwrap_or(sampler.maxzoom);
        sampler.profile(
            &body.line,
            zoom,
            body.densify_m.unwrap_or(50.0),
            body.threshold_m.unwrap_or(3.0),
        )
    })
    .await;
    match result {
        Ok(Ok(profile)) => Json(profile).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub struct ServerHandle {
    pub addr: SocketAddr,
    pub registry: Arc<Registry>,
}

impl ServerHandle {
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }
}

/// Bind and serve. Pass port 0 to let the OS choose - the chosen port is on the handle.
pub async fn start(port: u16, registry: Arc<Registry>) -> Result<ServerHandle> {
    start_with_root(port, registry, None).await
}

/// As [`start`], plus a catalog endpoint rooted at `output_root`.
pub async fn start_with_root(
    port: u16,
    registry: Arc<Registry>,
    output_root: Option<PathBuf>,
) -> Result<ServerHandle> {
    start_full(port, registry, output_root, None).await
}

/// As [`start_with_root`], plus a routing actor for the `/route` endpoint.
pub async fn start_full(
    port: u16,
    registry: Arc<Registry>,
    output_root: Option<PathBuf>,
    router: Option<crate::valhalla::routing::Router>,
) -> Result<ServerHandle> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    let base = format!("http://127.0.0.1:{}", addr.port());

    let state = ServerState {
        registry: registry.clone(),
        base,
        output_root,
        router: router.map(|r| std::sync::Arc::new(Mutex::new(r))),
    };
    let app = Router::new()
        .route("/sources", get(sources_handler))
        .route("/catalog", get(catalog_handler))
        .route("/steps", get(steps_handler))
        .route("/build-state/:area", get(build_state_handler))
        .route("/step-options/:step", get(step_options_handler))
        .route("/presets", get(presets_handler))
        .route("/plan", axum::routing::post(plan_handler))
        .route("/routing-status", get(routing_status_handler))
        .route("/route", axum::routing::post(route_handler))
        .route("/profile", axum::routing::post(profile_handler))
        .route("/tilejson/:area/:file", get(tilejson_handler))
        .route("/tiles/:area/:file/:z/:x/:y", get(tile_handler))
        // the webview is served from a different origin in dev, so the viewer needs CORS
        .layer(CorsLayer::permissive())
        .with_state(state);

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(ServerHandle { addr, registry })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;

    fn write_db(path: &std::path::Path, meta: &[(&str, &str)], tiles: &[(u8, u32, u32, Vec<u8>)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE metadata (name text, value text);
             CREATE TABLE tiles (zoom_level integer, tile_column integer,
               tile_row integer, tile_data blob);",
        )
        .unwrap();
        for (k, v) in meta {
            conn.execute("INSERT INTO metadata VALUES (?, ?)", (k, v)).unwrap();
        }
        for (z, x, row, blob) in tiles {
            conn.execute("INSERT INTO tiles VALUES (?, ?, ?, ?)", (z, x, row, blob)).unwrap();
        }
    }

    /// mbtiles rows are TMS (origin bottom-left); XYZ requests count from the top. At z=2 the
    /// four rows are 0..3, so XYZ y=1 must read TMS row 2.
    #[test]
    fn flips_tms_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mbtiles");
        write_db(&path, &[("format", "pbf")], &[(2, 1, 2, b"top-ish".to_vec())]);
        let src = Source::from_artifact(&catalog::probe(&path, "a"));
        assert_eq!(src.tile(2, 1, 1).unwrap().as_deref(), Some(&b"top-ish"[..]));
        assert_eq!(src.tile(2, 1, 2).unwrap(), None, "unflipped lookup must miss");
    }

    #[test]
    fn rejects_out_of_range_y() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mbtiles");
        write_db(&path, &[("format", "pbf")], &[(0, 0, 0, b"x".to_vec())]);
        let src = Source::from_artifact(&catalog::probe(&path, "a"));
        // at z=0 there is exactly one tile; y=1 would underflow the flip
        assert_eq!(src.tile(0, 0, 1).unwrap(), None);
        assert_eq!(src.tile(0, 0, 0).unwrap().as_deref(), Some(&b"x"[..]));
    }

    #[test]
    fn sniffs_gzip_rather_than_trusting_metadata() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08, 0x00]));
        // "RIFF" - a raw WebP tile, which the terrain archives store uncompressed
        assert!(!is_gzip(b"RIFF\x8a\x0c"));
        assert!(!is_gzip(&[0x1f]));
    }

    #[test]
    fn content_types_distinguish_mvt_from_mlt() {
        let dir = tempfile::tempdir().unwrap();
        let mvt = dir.path().join("a.mbtiles");
        write_db(&mvt, &[("format", "pbf")], &[]);
        assert_eq!(
            Source::from_artifact(&catalog::probe(&mvt, "a")).content_type(),
            "application/x-protobuf"
        );

        let mlt = dir.path().join("a_mlt.mbtiles");
        write_db(&mlt, &[("format", "application/vnd.maplibre-vector-tile")], &[]);
        assert_eq!(
            Source::from_artifact(&catalog::probe(&mlt, "a")).content_type(),
            "application/vnd.maplibre-vector-tile"
        );
    }

    #[tokio::test]
    async fn serves_tiles_over_http() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rhone-alpes.mbtiles");
        let gzipped = vec![0x1f, 0x8b, 0x08, 0x00, 0x01, 0x02];
        write_db(
            &path,
            &[("format", "pbf"), ("minzoom", "0"), ("maxzoom", "14"), ("bounds", "1,2,3,4")],
            &[(2, 1, 2, gzipped.clone())],
        );
        let registry = Arc::new(Registry::default());
        registry.set(
            "rhone-alpes/rhone-alpes.mbtiles".into(),
            Source::from_artifact(&catalog::probe(&path, "rhone-alpes")),
        );
        let server = start(0, registry).await.unwrap();
        let base = server.base_url();

        let res = reqwest::Client::new()
            .get(format!("{base}/tiles/rhone-alpes/rhone-alpes.mbtiles/2/1/1"))
            // reqwest would transparently decompress otherwise, hiding the header under test
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(res.headers()["content-type"], "application/x-protobuf");

        // a tile that does not exist is normal for sparse coverage
        let missing = reqwest::get(format!("{base}/tiles/rhone-alpes/rhone-alpes.mbtiles/2/0/0"))
            .await
            .unwrap();
        assert_eq!(missing.status(), 204);

        let unknown = reqwest::get(format!("{base}/tiles/nope/nope.mbtiles/0/0/0")).await.unwrap();
        assert_eq!(unknown.status(), 404);

        let body = reqwest::get(format!("{base}/tilejson/rhone-alpes/rhone-alpes.mbtiles"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let tj: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(tj["maxzoom"], 14);
        assert_eq!(tj["bounds"], serde_json::json!([1.0, 2.0, 3.0, 4.0]));
        assert!(tj["tiles"][0].as_str().unwrap().ends_with("/{z}/{x}/{y}"));
    }

    /// MapLibre templates often append an extension; the handler must tolerate `1.pbf`.
    #[tokio::test]
    async fn tolerates_an_extension_on_y() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.mbtiles");
        write_db(&path, &[("format", "pbf")], &[(2, 1, 2, b"hit".to_vec())]);
        let registry = Arc::new(Registry::default());
        registry.set("a/a.mbtiles".into(), Source::from_artifact(&catalog::probe(&path, "a")));
        let server = start(0, registry).await.unwrap();
        let res = reqwest::get(format!("{}/tiles/a/a.mbtiles/2/1/1.pbf", server.base_url()))
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
    }
}
