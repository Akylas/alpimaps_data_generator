# Third-party notices

Cairn's own code is [MIT](LICENSE). It builds on, bundles, or downloads the following. Each
keeps its own license; this file is a summary, not a substitute for theirs.

## Bundled with release builds

| Component | Role | License |
| --- | --- | --- |
| [Planetiler](https://github.com/onthegomap/planetiler) (fork: [farfromrefug/planetiler](https://github.com/farfromrefug/planetiler)) | vector tile generation, run as a subprocess | Apache-2.0 |
| [Planetiler OpenMapTiles profile](https://github.com/openmaptiles/planetiler-openmaptiles) (fork: [farfromrefug/planetiler-openmaptiles](https://github.com/farfromrefug/planetiler-openmaptiles)) | the basemap schema implementation | BSD-3-Clause (code), CC-BY 4.0 (schema design) |
| [Valhalla](https://github.com/valhalla/valhalla) | routing graph build and in-process routing | MIT |
| Valhalla's dependencies (protobuf, spatialite, SQLite, libcurl, zlib, LuaJIT, …) | carried in the app's `Frameworks/` | their own permissive licenses; see each library |

## Used by the build, not bundled

| Component | Role | License |
| --- | --- | --- |
| [VersaTiles](https://versatiles.org) CLI | bathymetry / low-zoom land cover | MIT |
| [tippecanoe](https://github.com/felt/tippecanoe) (fork: [farfromrefug/tippecanoe](https://github.com/farfromrefug/tippecanoe)) | bathymetry tiles | BSD-2-Clause |
| [GDAL](https://gdal.org) | legacy pipeline only | MIT |
| [rio-rgbify](https://github.com/mapbox/rio-rgbify) (fork: [farfromrefug/rio-rgbify](https://github.com/farfromrefug/rio-rgbify)) | legacy pipeline only | MIT |
| [prime_server](https://github.com/kevinkreiser/prime_server) | legacy pipeline only | BSD-2-Clause |

## Rust and JavaScript dependencies

Listed in [`Cargo.lock`](Cargo.lock) and [`package-lock.json`](package-lock.json). The direct
ones include [Tauri](https://tauri.app) (MIT / Apache-2.0), [Svelte](https://svelte.dev) (MIT),
[MapLibre GL JS](https://maplibre.org) (BSD-3-Clause), [axum](https://github.com/tokio-rs/axum)
(MIT), [tokio](https://tokio.rs) (MIT), [PMTiles](https://github.com/protomaps/PMTiles) readers
and writers (BSD-3-Clause / MIT / Apache-2.0), and [`@maplibre/mlt`](https://github.com/maplibre/maplibre-tile-spec) (MIT).

Layer handling in the map view is adapted from
[mbview-rs](https://github.com/farfromrefug/mbview-rs).

## Data

Cairn ships **no map data**. What it downloads or generates carries its source's terms, and
those apply to what you publish:

| Data | Source | Terms |
| --- | --- | --- |
| OpenStreetMap extracts | [Geofabrik](https://download.geofabrik.de) | [ODbL 1.0](https://opendatacommons.org/licenses/odbl/) — © OpenStreetMap contributors. Attribution and share-alike apply to the tiles you build |
| Elevation (SRTM `.hgt`) | Skadi mirror | public domain (NASA/USGS) |
| Elevation (default `config/sources.json`) | [Mapterhorn](https://mapterhorn.com) | see their [attribution page](https://mapterhorn.com) |
| Elevation (optional IGN RGE ALTI) | IGN | [Licence Ouverte 2.0](https://www.etalab.gouv.fr/licence-ouverte-open-licence/) |
| Natural Earth (coastlines, bathymetry bands, low-zoom basemap layers) | [Natural Earth](https://www.naturalearthdata.com) | public domain |
| ESA WorldCover land cover, via the [VersaTiles](https://versatiles.org) landcover container | the bathymap step | [CC BY 4.0](https://worldcover2021.esa.int/) — © ESA WorldCover project |

"OpenMapTiles" is a trademark of OpenMapTiles Association; Cairn implements the open schema
and is not affiliated with or endorsed by it.
