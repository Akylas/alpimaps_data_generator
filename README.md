# alpimaps_data_generator

Builds the offline map data for [AlpiMaps](https://github.com/Akylas/alpimaps): vector basemap
and route tiles, terrain-RGB elevation tiles, and a Valhalla routing package — for any area
Geofabrik publishes.

The outputs are ordinary MBTiles and a Valhalla tar, so they work just as well with
`tileserver-gl`, QGIS, MapLibre, or anything else that reads them.

## What it produces

| Output | What it is |
| --- | --- |
| `${AREA}.mbtiles` | OpenMapTiles-schema vector basemap, z0–14 |
| `${AREA}_routes.mbtiles` | hiking and cycling route relations, kept in their own archive |
| `${AREA}_terrain.mbtiles` | terrain-RGB elevation, for shaded relief and 3D |
| `valhalla_tiles.tar` | routing graph, with elevation baked in |

## Requirements

Java 21+ and Rust. Then the native libraries the elevation and routing steps need:

```shell
# macOS
brew install aria2 gdal autoconf automake zmq czmq spatialite-tools luajit

# Ubuntu
sudo add-apt-repository ppa:ubuntugis/ubuntugis-unstable
sudo apt install -y aria2 gdal-bin autoconf automake pkg-config libtool make gcc g++ cmake \
  curl unzip jq protobuf-compiler vim-common locales libcurl4-openssl-dev zlib1g-dev \
  liblz4-dev libprotobuf-dev spatialite-tools luajit
```

Run `./setup.sh` once to fetch the submodules and build the vendored tools.

## Quick start

```shell
cd cairn && cargo build --release -p cairn-cli   # target/release/cairn
```

```shell
export AREA=rhone-alpes

cairn download --area $AREA        # OSM extract from Geofabrik
cairn basemap  --area $AREA        # vector basemap
cairn routes   --area $AREA        # route relations
cairn elevation --area $AREA       # .hgt tiles
cairn terrain  --area $AREA        # terrain-RGB
cairn valhalla-tiles --area $AREA  # routing graph
cairn package  --area $AREA        # Valhalla package
```

Every step clips to the area's own boundary, downloading the `.poly` from Geofabrik the first
time it needs it. Pass `--poly-shape` to clip to something else.

`cairn state --area $AREA` reports which outputs exist; `cairn state --area $AREA clear <step>`
removes one.

## Tuning

Each step's defaults reproduce the builds documented in
[docs/pipeline-reference.md](docs/pipeline-reference.md) — an untouched run gives you the same
bytes as the reference pipeline. Tests parse that document and compare it against the defaults,
so the two cannot drift apart.

```shell
cairn options basemap            # every flag, with what it costs
cairn options basemap --presets  # and the option sets that ship
```

Two presets ship per planetiler step: `measured` (the default) and `stock` (planetiler's own
defaults, as a comparison baseline). Override anything with `-o`:

```shell
cairn basemap --area $AREA -o transportation_surface_detail=false
cairn basemap --area $AREA --preset stock
```

Some flags worth knowing about, all measured on a rhone-alpes build:

| Flag | Effect |
| --- | --- |
| `max_point_buffer=4` | caps the buffer on point layers. `place` declares 256px — nine times a tile's own area — so leaving it uncapped costs tens of MB |
| `landcover_merge_maxzoom` | merges landcover polygons at z14, where most of the bytes are. −19% of that layer |
| `transportation_surface_detail` | road surface and tracktype on every class, +0.84%. Reaches 73% of tracks — the difference between riding and pushing |
| `water_pool_tolerance=1` | simplifies swimming pools, which carry ~99 vertices each for a shape a pixel across |
| `drop_redundant_name_int` | omits `name_int` where it duplicates `name`, −0.88% |

## The desktop app

[`cairn/`](cairn/) is also a desktop app over the same code: pick an area, tick the steps, watch
the progress, and inspect the resulting tiles on a map.

```shell
cd cairn && npm install && npm run tauri dev
```

## Repository layout

| Path | |
| --- | --- |
| `cairn/` | the app and CLI — `core/` is the shared pipeline, `cli/`, `src-tauri/`, `src/` |
| `scripts/` | the original Python and shell pipeline, still runnable |
| `planetiler/` | vector tile generation, as a fork with this project's schema changes |
| `valhalla/`, `prime_server/` | routing |
| `docs/pipeline-reference.md` | every step as a raw tool invocation |

## Related

The planetiler fork carries the schema work this project depends on — POI selection, the
landcover and route tuning, road surface detail. Changes land in
[farfromrefug/planetiler-openmaptiles](https://github.com/farfromrefug/planetiler-openmaptiles)
and are picked up here as a submodule bump.
