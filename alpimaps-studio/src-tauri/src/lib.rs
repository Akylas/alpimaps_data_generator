//! Tauri shell. Deliberately thin: everything with logic lives in `studio-core`, which has no
//! Tauri dependency and therefore tests in seconds rather than behind a webview build.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use studio_core::catalog::{self, Area, TileStats};
use studio_core::elevation::{Profile, TerrainSampler};
use studio_core::tileserver::{self, Registry, Source};
use studio_core::settings::Settings;
use studio_core::presets::{Preset, PresetStore};
use studio_core::valhalla::{package, routing};
use studio_core::steps::options::{self, OptionDef};
use studio_core::steps::planetiler::{run_cancellable, PlanetilerJob, Schema};
use studio_core::steps::{plan, StepEvent, StepId, ALL_STEPS};
use std::collections::BTreeMap;
use studio_core::toolchain::{self, JavaInstall};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;

/// Cancel signal for the run in flight, if any.
///
/// A broadcast rather than a plain channel because one run spans several steps: each step
/// subscribes for its own lifetime, and a single cancel reaches whichever step is executing.
#[derive(Default)]
struct Running(Mutex<Option<tokio::sync::broadcast::Sender<()>>>);

/// Settings, kept in memory and written through on every change.
struct Config {
    path: PathBuf,
    settings: Mutex<Settings>,
}

impl Config {
    fn load(app: &AppHandle) -> Self {
        let dir = app.path().app_config_dir().unwrap_or_else(|_| PathBuf::from("."));
        let path = dir.join("settings.json");
        // cwd is the least-wrong default for the repo root; the user retargets it in Settings
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let settings = Settings::load_or_default(&path, cwd).unwrap_or_default();
        Self { path, settings: Mutex::new(settings) }
    }

    fn get(&self) -> Result<Settings, String> {
        Ok(self.settings.lock().map_err(|_| "settings lock poisoned")?.clone())
    }
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    done: u64,
    total: Option<u64>,
}

fn managed_root(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app data dir: {e}"))?
        .join("jre");
    Ok(dir)
}

#[tauri::command]
async fn detect_java(app: AppHandle) -> Result<JavaInstall, String> {
    let root = managed_root(&app)?;
    toolchain::find(None, &root)
        .await
        .ok_or_else(|| format!("no Java {}+ on PATH, in JAVA_HOME, or downloaded", toolchain::MIN_JAVA))
}

#[tauri::command]
async fn download_java(app: AppHandle) -> Result<JavaInstall, String> {
    let root = managed_root(&app)?;
    let emitter = app.clone();
    toolchain::download(&root, move |done, total| {
        let _ = emitter.emit("jre-download", DownloadProgress { done, total });
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_settings(config: State<'_, Config>) -> Result<Settings, String> {
    config.get()
}

#[tauri::command]
fn save_settings(config: State<'_, Config>, settings: Settings) -> Result<Settings, String> {
    settings.save(&config.path).map_err(|e| e.to_string())?;
    let mut slot = config.settings.lock().map_err(|_| "settings lock poisoned")?;
    *slot = settings.clone();
    Ok(settings)
}

/// Scan the configured output root. Cheap - reads metadata only, never the tiles table.
#[tauri::command]
fn list_areas(config: State<'_, Config>) -> Result<Vec<Area>, String> {
    let settings = config.get()?;
    catalog::discover(&settings.output_root).map_err(|e| e.to_string())
}

/// Walks the whole archive, so it is deliberately a separate call the UI makes on demand
/// rather than something `list_areas` pays for. About 18s for 1.4 GB of output.
#[tauri::command]
async fn artifact_stats(path: String) -> Result<TileStats, String> {
    tokio::task::spawn_blocking(move || catalog::tile_stats(Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct Comparison {
    zooms: Vec<catalog::ZoomDelta>,
    metadata: catalog::MetadataDiff,
    layers: Vec<catalog::LayerDiff>,
    size_a: u64,
    size_b: u64,
}

#[tauri::command]
async fn compare_artifacts(area: String, a: String, b: String) -> Result<Comparison, String> {
    tokio::task::spawn_blocking(move || -> Result<Comparison, String> {
        let (pa, pb) = (PathBuf::from(&a), PathBuf::from(&b));
        let (art_a, art_b) = (catalog::probe(&pa, &area), catalog::probe(&pb, &area));
        let stats_a = catalog::tile_stats(&pa).map_err(|e| e.to_string())?;
        let stats_b = catalog::tile_stats(&pb).map_err(|e| e.to_string())?;
        Ok(Comparison {
            zooms: catalog::diff_zooms(&stats_a, &stats_b),
            metadata: catalog::diff_metadata(&art_a, &art_b),
            layers: catalog::diff_layers(&art_a, &art_b),
            size_a: art_a.size_bytes,
            size_b: art_b.size_bytes,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The tile server plus the registry it reads from. Started on demand, once.
#[derive(Default)]
struct Tiles {
    registry: Arc<Registry>,
    base_url: Mutex<Option<String>>,
}

/// Start the tile server if it is not already up, then (re)register every discovered artifact.
///
/// Registration is redone on every call so a rebuild is picked up without a restart; the server
/// itself is bound once because rebinding would change the port under the running map.
#[tauri::command]
async fn start_tiles(tiles: State<'_, Tiles>, config: State<'_, Config>) -> Result<String, String> {
    let settings = config.get()?;
    let registry = tiles.registry.clone();

    let base = {
        let existing = tiles.base_url.lock().map_err(|_| "tiles lock poisoned")?.clone();
        match existing {
            Some(url) => url,
            None => {
                let handle = tileserver::start_with_root(
                    0,
                    registry.clone(),
                    Some(settings.output_root.clone()),
                )
                .await
                .map_err(|e| e.to_string())?;
                let url = handle.base_url();
                *tiles.base_url.lock().map_err(|_| "tiles lock poisoned")? = Some(url.clone());
                url
            }
        }
    };

    let areas = catalog::discover(&settings.output_root).map_err(|e| e.to_string())?;
    registry.clear();
    for area in &areas {
        for art in &area.artifacts {
            // routing packages hold graph tiles, not map tiles - nothing to render
            if art.probe_error.is_some() || art.format == catalog::TileFormat::Gph3 {
                continue;
            }
            registry.set(format!("{}/{}", area.name, art.file_name), Source::from_artifact(art));
        }
    }
    Ok(base)
}

/// camelCase because the caller is JavaScript. Without this the `densify_m` / `threshold_m`
/// fields silently fall back to their serde defaults instead of using what the UI sent - a
/// wrong-but-plausible profile rather than an error.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileRequest {
    path: String,
    line: Vec<[f64; 2]>,
    #[serde(default)]
    zoom: Option<u8>,
    #[serde(default = "default_densify")]
    densify_m: f64,
    #[serde(default = "default_threshold")]
    threshold_m: f64,
}

fn default_densify() -> f64 {
    50.0
}

/// 3 m, not 0. At the terrain's 1 m vertical quantisation a flat traverse dithers between two
/// adjacent levels, and summing raw deltas turns that into phantom climb.
fn default_threshold() -> f64 {
    3.0
}

#[tauri::command]
async fn elevation_profile(req: ProfileRequest) -> Result<Profile, String> {
    tokio::task::spawn_blocking(move || -> Result<Profile, String> {
        let mut sampler = TerrainSampler::open(Path::new(&req.path)).map_err(|e| e.to_string())?;
        let zoom = req.zoom.unwrap_or(sampler.maxzoom);
        sampler
            .profile(&req.line, zoom, req.densify_m, req.threshold_m)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct StepInfo {
    id: StepId,
    label: &'static str,
    deps: Vec<StepId>,
    implemented: bool,
}

/// A routing actor, kept alive between requests along with the tile directory it was opened on.
///
/// Creating one is cheap, but its graph reader caches tiles, so holding it makes the second and
/// later routes noticeably faster than the first.
#[derive(Default)]
struct Routing(Mutex<Option<(PathBuf, routing::Router)>>);

#[derive(Serialize)]
struct RoutingStatus {
    available: bool,
    /// The tile directory in use, once one has been resolved.
    tile_dir: Option<String>,
}

#[tauri::command]
fn routing_status(routing_state: State<'_, Routing>) -> RoutingStatus {
    let tile_dir = routing_state
        .0
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|(dir, _)| dir.display().to_string()));
    RoutingStatus { available: routing::available(), tile_dir }
}

/// Resolve a tile directory for an area, unpacking its package when needed.
///
/// The package is what actually ships, so it is preferred over the intermediate
/// `valhalla_tiles/` build output: routing against it exercises the artefact users get. The
/// unpack is cached by the package's size and modification time, so it happens once per build
/// rather than once per request.
fn tile_dir_for(settings: &Settings, area: &str, cache_root: &Path) -> Result<PathBuf, String> {
    let package_path = settings.area_dir(area).join(format!("{area}.vtiles"));
    if !package_path.is_file() {
        let fallback = settings.repo_root.join("valhalla_tiles");
        return if fallback.is_dir() {
            Ok(fallback)
        } else {
            Err(format!("no {} and no valhalla_tiles/ to fall back on", package_path.display()))
        };
    }

    let stamp = std::fs::metadata(&package_path)
        .ok()
        .map(|m| {
            let modified = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{}-{}", m.len(), modified)
        })
        .unwrap_or_else(|| "unknown".into());
    let unpacked = cache_root.join(format!("{area}-{stamp}"));
    if unpacked.is_dir() {
        return Ok(unpacked);
    }

    // a stale unpack of the same area would otherwise accumulate on every rebuild
    if let Ok(entries) = std::fs::read_dir(cache_root) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&format!("{area}-")) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    package::unpack(&package_path, &unpacked).map_err(|e| e.to_string())?;
    Ok(unpacked)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteRequest {
    area: String,
    /// `[lon, lat]` pairs in visiting order.
    locations: Vec<[f64; 2]>,
    costing: String,
}

/// Route through the given waypoints, returning Valhalla's JSON response verbatim.
#[tauri::command]
async fn valhalla_route(
    app: AppHandle,
    config: State<'_, Config>,
    routing_state: State<'_, Routing>,
    req: RouteRequest,
) -> Result<String, String> {
    if !routing::available() {
        return Err("this build has no Valhalla linked".into());
    }
    let settings = config.get()?;
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("valhalla");
    std::fs::create_dir_all(&cache_root).map_err(|e| e.to_string())?;

    let tile_dir = tile_dir_for(&settings, &req.area, &cache_root)?;
    let template = settings.repo_root.join("valhalla.json");
    let request_json = routing::RouteRequest {
        locations: req.locations,
        costing: req.costing,
    }
    .to_json();

    let mut guard = routing_state.0.lock().map_err(|_| "routing lock poisoned")?;
    let needs_open = guard.as_ref().map(|(dir, _)| dir != &tile_dir).unwrap_or(true);
    if needs_open {
        let router = routing::Router::open(&template, &tile_dir).map_err(|e| e.to_string())?;
        *guard = Some((tile_dir, router));
    }
    let (_, router) = guard.as_mut().expect("just opened");
    router.route(&request_json).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_steps() -> Vec<StepInfo> {
    ALL_STEPS
        .iter()
        .map(|s| StepInfo {
            id: *s,
            label: s.label(),
            deps: s.deps().to_vec(),
            // hillshade, the OSM download and valhalla_build_tiles are still shell scripts;
            // the runner says so rather than pretending to have run them
            implemented: s.is_implemented(),
        })
        .collect()
}

#[tauri::command]
fn step_options(step: StepId) -> Vec<OptionDef> {
    match step {
        StepId::Routes => options::routes_options(),
        StepId::Basemap => options::basemap_options(),
        _ => options::planetiler_common(),
    }
}

#[tauri::command]
fn plan_steps(steps: Vec<StepId>) -> Vec<StepId> {
    plan(&steps)
}

fn presets_path(config: &Config) -> PathBuf {
    config.path.with_file_name("presets.json")
}

#[tauri::command]
fn list_presets(config: State<'_, Config>) -> Result<Vec<Preset>, String> {
    let mut store = PresetStore::load_or_default(&presets_path(&config)).map_err(|e| e.to_string())?;
    // built-ins are merged in rather than written to disk, so they keep improving with the app
    // while a user preset of the same name still wins
    let mut merged = PresetStore::default();
    for p in studio_core::presets::builtin() {
        merged.upsert(p);
    }
    for p in store.presets.drain(..) {
        merged.upsert(p);
    }
    Ok(merged.presets)
}

#[tauri::command]
fn save_preset(config: State<'_, Config>, preset: Preset) -> Result<(), String> {
    let path = presets_path(&config);
    let mut store = PresetStore::load_or_default(&path).map_err(|e| e.to_string())?;
    store.upsert(preset);
    store.save(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_preset(config: State<'_, Config>, step: StepId, name: String) -> Result<bool, String> {
    let path = presets_path(&config);
    let mut store = PresetStore::load_or_default(&path).map_err(|e| e.to_string())?;
    let removed = store.remove(step, &name);
    store.save(&path).map_err(|e| e.to_string())?;
    Ok(removed)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunRequest {
    area: String,
    steps: Vec<StepId>,
    #[serde(default)]
    values: BTreeMap<StepId, BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    schema_yaml: Option<String>,
    #[serde(default)]
    jar: Option<String>,
}

/// Run a selection of steps, in dependency order.
///
/// Steps run one at a time on purpose. The two planetiler steps write into a temp tree, and
/// running them concurrently is what deleted each other's sort chunks - each still gets its own
/// tmpdir, but serialising removes the whole class of problem.
#[tauri::command]
async fn run_steps(
    app: AppHandle,
    running: State<'_, Running>,
    config: State<'_, Config>,
    req: RunRequest,
) -> Result<Vec<StepId>, String> {
    let settings = config.get()?;
    let java = detect_java(app.clone()).await?;
    let jar = req
        .jar
        .map(PathBuf::from)
        .or(settings.planetiler_jar.clone())
        .ok_or("no planetiler jar configured")?;
    let schema = match req.schema_yaml.as_deref() {
        Some(path) if !path.is_empty() => Schema::Yaml { path: PathBuf::from(path) },
        _ => Schema::OpenMapTiles,
    };

    let ordered = plan(&req.steps);
    let (cancel_tx, _) = tokio::sync::broadcast::channel(4);
    {
        let mut slot = running.0.lock().map_err(|_| "runner lock poisoned")?;
        if slot.is_some() {
            return Err("a build is already running".into());
        }
        *slot = Some(cancel_tx.clone());
    }

    let (tx, mut rx) = mpsc::channel::<StepEvent>(512);
    let emitter = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = emitter.emit("step", &event);
        }
    });

    let mut completed = Vec::new();
    for step in ordered {
        if step == StepId::TerrainRgb {
            match run_terrain(&settings, &req.area, tx.clone()).await {
                Ok(()) => completed.push(step),
                Err(e) => {
                    let _ = tx.send(StepEvent::Log { step, line: format!("ERROR: {e}") }).await;
                    break;
                }
            }
            continue;
        }
        if step == StepId::ValhallaPackage {
            match run_valhalla_package(&settings, &req.area, tx.clone()).await {
                Ok(()) => completed.push(step),
                Err(e) => {
                    let _ = tx.send(StepEvent::Log { step, line: format!("ERROR: {e}") }).await;
                    break;
                }
            }
            continue;
        }
        if !step.is_planetiler() {
            let _ = tx
                .send(StepEvent::Log {
                    step,
                    line: format!("{} is not implemented in the app yet - skipped", step.label()),
                })
                .await;
            continue;
        }
        let values = req.values.get(&step).cloned().unwrap_or_default();
        let defs = match step {
            StepId::Routes => options::routes_options(),
            _ => options::basemap_options(),
        };
        let mut extra = vec!["--download".into(), format!("--area={}", req.area), "--force".into()];
        extra.extend(options::to_args(&defs, &values));

        let suffix = if step == StepId::Routes { "_routes" } else { "" };
        let job = PlanetilerJob {
            step,
            area: req.area.clone(),
            java: java.path.clone(),
            jar: jar.clone(),
            schema: schema.clone(),
            heap_mb: settings.heap_mb,
            output: settings
                .area_dir(&req.area)
                .join(format!("{}{suffix}.mbtiles", req.area)),
            tmp_dir: settings.run_tmp_dir(&format!("{}-{:?}", req.area, step)),
            extra_args: extra,
            working_dir: settings.repo_root.clone(),
            log_interval: settings.log_interval.clone(),
        };

        // bridge the run-wide broadcast onto this step's own channel; the task ends when the
        // signal arrives or when the broadcast sender is dropped at the end of the run
        let mut subscription = cancel_tx.subscribe();
        let (step_tx, step_cancel) = mpsc::channel(1);
        tokio::spawn(async move {
            if subscription.recv().await.is_ok() {
                let _ = step_tx.send(()).await;
            }
        });

        let ok = run_cancellable(job, tx.clone(), step_cancel)
            .await
            .map_err(|e| e.to_string())?;
        if !ok {
            break;
        }
        completed.push(step);
    }

    if let Ok(mut slot) = app.state::<Running>().0.lock() {
        *slot = None;
    }
    Ok(completed)
}

/// Build the terrain pyramid from `.hgt` sources.
///
/// Reads only SRTM `.hgt`, which needs no projection library. The GeoTIFF raster sources in
/// `sources.json` (the IGN 5 m data, EPSG:2154) are not read here yet, so this produces the
/// tilezen-resolution pyramid rather than the IGN-composited one.
async fn run_terrain(
    settings: &Settings,
    area: &str,
    tx: mpsc::Sender<StepEvent>,
) -> Result<(), String> {
    use studio_core::terrain::{render, source};

    let step = StepId::TerrainRgb;
    let _ = tx.send(StepEvent::Started { step, area: area.to_string() }).await;

    let sources_json = settings.sources_json.clone();
    let hgt_dir = settings.elevation_tiles_dir.clone();
    let output = settings.area_dir(area).join(format!("{area}_terrain.mbtiles"));
    let opts = render::TerrainOptions::default();

    // bounds come from the area's basemap when there is one, so terrain matches its extent
    let bounds = catalog::discover(&settings.output_root)
        .ok()
        .and_then(|areas| {
            areas
                .into_iter()
                .find(|a| a.name == area)?
                .artifacts
                .iter()
                .find(|a| a.kind == catalog::ArtifactKind::Basemap)?
                .bounds
                .clone()
        })
        .and_then(|b| {
            let p: Vec<f64> = b.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            (p.len() == 4).then(|| (p[0], p[1], p[2], p[3]))
        })
        .ok_or("no basemap bounds to derive the terrain extent from")?;

    let name = format!("{area}_terrain");
    let progress = tx.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        // sources.json is the pipeline's own definition of what to read, in priority order;
        // fall back to the bare elevation directory when it is absent
        let specs = source::read_specs(&sources_json).unwrap_or_else(|_| {
            vec![source::SourceSpec {
                name: "elevation_tiles".into(),
                kind: "valhalla".into(),
                path: hgt_dir.clone(),
                clamp_min: Some(-10.0),
            }]
        });
        let (mut source, skipped) = source::CompositeSource::open(&specs).map_err(|e| e.to_string())?;
        for note in skipped {
            let _ = progress.blocking_send(StepEvent::Log { step, line: format!("skipped {note}") });
        }
        let conn = render::create_archive(&output, &name, &opts, bounds).map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("INSERT INTO tiles VALUES (?, ?, ?, ?)")
            .map_err(|e| e.to_string())?;

        for zoom in opts.minzoom..=opts.maxzoom {
            let (x0, y0, x1, y1) = render::tile_range(zoom, bounds);
            let total = ((x1 - x0 + 1) as u64) * ((y1 - y0 + 1) as u64);
            let _ = progress.blocking_send(StepEvent::Phase { step, name: format!("z{zoom}") });
            let mut done = 0u64;
            for x in x0..=x1 {
                for y in y0..=y1 {
                    done += 1;
                    let Some(rgb) = render::render_tile(&mut source, zoom, x, y, &opts) else {
                        continue;
                    };
                    let webp = render::to_webp(&rgb, opts.tile_size).map_err(|e| e.to_string())?;
                    // mbtiles rows are TMS, counting up from the south
                    let tms = (1u32 << zoom) - 1 - y;
                    stmt.execute((zoom, x, tms, &webp)).map_err(|e| e.to_string())?;
                    if done % 16 == 0 {
                        let _ = progress.blocking_send(StepEvent::Progress {
                            step,
                            label: format!("z{zoom}"),
                            percent: ((done * 100) / total.max(1)).min(100) as u8,
                        });
                    }
                }
            }
        }
        drop(stmt);
        conn.execute_batch(
            "CREATE UNIQUE INDEX tile_index ON tiles (zoom_level, tile_column, tile_row)",
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;

    let _ = tx
        .send(StepEvent::Finished { step, ok: true, elapsed: None, outputs: vec![] })
        .await;
    Ok(())
}

/// Compress a Valhalla tile directory into a `.vtiles` package.
async fn run_valhalla_package(
    settings: &Settings,
    area: &str,
    tx: mpsc::Sender<StepEvent>,
) -> Result<(), String> {
    use studio_core::valhalla::package::{self, Compression, PackageOptions};

    let step = StepId::ValhallaPackage;
    let _ = tx.send(StepEvent::Started { step, area: area.to_string() }).await;

    let tile_dir = settings.repo_root.join("valhalla_tiles");
    let output = settings.area_dir(area).join(format!("{area}.vtiles"));
    // reuse the tile list of the previous package when there is one; otherwise there is no
    // tilemask to work from and the step cannot pick tiles on its own yet
    let tiles = package::tiles_in(&output)
        .map_err(|_| "no existing package to take a tile list from; build one with the script first")?;

    let opts = PackageOptions {
        package_id: area.to_string(),
        tile_dir,
        output,
        compression: Compression::Zopfli,
    };
    let progress = tx.clone();
    let report = tokio::task::spawn_blocking(move || {
        package::build(&opts, &tiles, |done, total| {
            let _ = progress.blocking_send(StepEvent::Progress {
                step,
                label: "tiles".into(),
                percent: ((done * 100) / total.max(1)).min(100) as u8,
            });
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let _ = tx
        .send(StepEvent::Log {
            step,
            line: format!(
                "{} tiles, {:.1} MiB compressed (ratio {:.3}), {} missing",
                report.tiles_written,
                report.compressed_bytes as f64 / 1_048_576.0,
                report.ratio(),
                report.tiles_missing
            ),
        })
        .await;
    let _ = tx
        .send(StepEvent::Finished { step, ok: true, elapsed: None, outputs: vec![] })
        .await;
    Ok(())
}

#[tauri::command]
fn cancel_run(running: State<'_, Running>) -> Result<(), String> {
    let sender = {
        let slot = running.0.lock().map_err(|_| "runner lock poisoned")?;
        slot.clone()
    };
    match sender {
        // no subscriber means the current step already finished; that is not an error
        Some(tx) => tx.send(()).map(|_| ()).or(Ok(())),
        None => Err("nothing running".into()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Running::default())
        .manage(Tiles::default())
        .manage(Routing::default())
        .setup(|app| {
            app.manage(Config::load(&app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            detect_java,
            download_java,
            cancel_run,
            get_settings,
            save_settings,
            list_areas,
            artifact_stats,
            compare_artifacts,
            start_tiles,
            elevation_profile,
            routing_status,
            valhalla_route,
            list_steps,
            step_options,
            plan_steps,
            list_presets,
            save_preset,
            delete_preset,
            run_steps
        ])
        .run(tauri::generate_context!())
        .expect("error while running AlpiMaps Studio");
}
