
### Introduction

That repo explains how to generate data to be used with [AlpiMaps](https://github.com/Akylas/alpimaps).
It can also be used to generate mbtiles to be used with other projects like `tileserver-gl`, `qgis` ...

Everything below is the pipeline run by hand, which is still the reference for what each step
does. [`cairn/`](cairn/) packages the same pipeline two other ways: **Cairn**, a desktop app,
and `cairn`, a command line that replaces the scripts. Same code underneath, so a build started
any of the three ways produces the same bytes.

| By hand | With the CLI |
| --- | --- |
| `scripts/download-osm.py geofabrik ${AREA}` | `cairn download --area ${AREA}` |
| `valhalla_build_elevation -v -d -b $BOUNDS -o ./elevation_tiles` | `cairn elevation --area ${AREA}` (native, no Python) |
| `java -jar $PLANETILER_JAR --area=${AREA} ... --exclude_layers=route` | `cairn basemap --area ${AREA}` |
| `java -jar $PLANETILER_JAR --area=${AREA} ... --only_layers=route` | `cairn routes --area ${AREA}` |
| `scripts/build_terrain_rgb.py --sources sources.json ...` | `cairn terrain --area ${AREA} --poly-shape $POLY` |
| `scripts/build_hillshades.sh ...` | `cairn hillshade --area ${AREA} --poly-shape $POLY` |
| `valhalla_build_tiles -c valhalla.json data/sources/...osm.pbf` | `cairn valhalla-tiles --area ${AREA}` |
| `scripts/build_valhalla_package.py --id $AREA --poly $POLY ...` | `cairn package --area ${AREA} --poly $POLY` |

The CLI takes every path as a flag and runs exactly the step you name; `--dry-run` prints the
command or tile list it would use. `cairn state --area ${AREA}` reports which outputs already
exist, and `state ... clear <step>` deletes one. Contours are not covered: they are drawn from the RGB terrain tiles now.

```shell
cd cairn && cargo build --release -p cairn-cli   # target/release/cairn
```

### macos

```shell
brew install aria2 gdal autoconf automake zmq czmq spatialite-tools luajit
pip3 install gdal

```

### ubuntu

```shell
sudo add-apt-repository ppa:ubuntugis/ubuntugis-unstable
sudo apt install -y aria2 gdal-bin autoconf automake pkg-config libtool make gcc g++ lcov cmake make libtool pkg-config g++ gcc curl unzip jq lcov protobuf-compiler vim-common locales libcurl4-openssl-dev zlib1g-dev liblz4-dev libprotobuf-dev spatialite-tools luajit
```
You'll also need venv for python (package depending on your python3 version but something like `sudo apt install -y python3.10-venv`)

You ll need to install

## prepare

Now you need to run `./setup.sh` at leat once

after that if you dont want to use the full build script you need to run `source ./env.sh` to ensure env variables are set


## Building

First download the poly of the area  you want from geofabrick or find the bounds you want

* polyzoom: zoom used to compute wanted tiles from poly-shape. The bigger the slower to compute but also the more defined is your zone
* elevation_tiles: folder where to store elevation_tiles use by valhalla and to generate tiffs


First we need the poly / pbf from that region

```shell
export AREA=italy
python ./scripts/download-osm.py --poly $AREA
export POLY=$AREA.poly
java -jar $PLANETILER_JAR  --only-download --area=$AREA
valhalla_build_config --mjolnir-tile-dir ${PWD}/valhalla_tiles --mjolnir-tile-extract ${PWD}/valhalla_tiles.tar --mjolnir-timezone ${PWD}/valhalla_tiles/timezones.sqlite --mjolnir-admin ${PWD}/valhalla_tiles/admins.sqlite --additional-data-elevation ${PWD}/elevation_tiles > valhalla.json 
```

## # First generate the mbtiles with Planetiler
You can change the languages parameter to your need ( like `en,fr`)

There you have multiple choices. Either build only using the area you want. But you will end up with half-filled tiles on area bounds
```shell
java -Xmx32g -jar $PLANETILER_JAR  --download --area=${AREA} --languages="" --force --compact-db --transportation-name-limit-merge -exclude_layers=route --nodemap-type=sparsearray --mbtiles=${OUTPUT_DIR}/${AREA}/${AREA}.mbtiles --polygon=$POLY --max-point-buffer=4  --transportation_z13_paths --mlt-shared-dict --parallel-tmp-io --simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 --landcover_tolerance_z11_13=1.05 --landcover_drop_redundant_subclass=true --landcover_merge_maxzoom=true --drop_redundant_name_int=true --transportation_surface_detail=true --water_pool_tolerance=1
```
Or build using a "parent" area. For example i will always use europe as i mostly build europe countries

```shell
java -Xmx32g -jar $PLANETILER_JAR  --download --area=europe --languages="" --force --compact-db --transportation-name-limit-merge -exclude_layers=route --nodemap-type=sparsearray --mbtiles=${OUTPUT_DIR}/${AREA}/${AREA}.mbtiles --polygon=$POLY --max-point-buffer=4 --transportation_z13_paths --mlt-shared-dict --parallel-tmp-io --simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 --landcover_tolerance_z11_13=1.05 --landcover_drop_redundant_subclass=true --landcover_merge_maxzoom=true --drop_redundant_name_int=true --transportation_surface_detail=true --water_pool_tolerance=1 --skip_filled_tiles
```

If you want to generate low level world map:
```shell
java -Xmx32g -jar $PLANETILER_JAR  --download --area=planet --languages=fr,en --force --transportation-name-limit-merge --compact-db --only_layers=place,park,boundary,mountain_peak,transportation,transportation_name,water,waterway,water_name,landcover,landcover_name,landuse --maxzoom=7 --nodemap-type=sparsearray --mbtiles=${OUTPUT_DIR}/world.mbtiles --max-point-buffer=4 --mlt-shared-dict --parallel-tmp-io --simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 --landcover_tolerance_z11_13=1.05 --landcover_drop_redundant_subclass=true --landcover_merge_maxzoom=true --drop_redundant_name_int=true --skip_filled_tiles


## # Generate routes mbtiles

```shell
java -Xmx32g -jar $PLANETILER_JAR  --download --area=${AREA} --languages="" --force --compact-db --transportation-name-limit-merge -only_layers=route --nodemap-type=sparsearray --mbtiles=${OUTPUT_DIR}/${AREA}/${AREA}_routes.mbtiles --polygon=$POLY --max-point-buffer=4 --mlt-shared-dict --parallel-tmp-io --route_road_tolerance=true --route_extent_digits=2 --route_symbol_id=true \
--simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25
```

## # Generate area tif

Now generate the tif of the area you want. It is best to use polyzoom as the min zoom you want for hillshades / contours. This ensure you wont get half filled tiles. Though the process will be slower and the tif bigger
We use a small polyzoom (5) to avoid half filled tiles. You can raise it for faster build but you ll have half filled tiles in hillshades or contours (mostly hillshade as contours is starting at zoom 11 here)
If you want to have more defined tif over (that you would get from another source) you can use the `--overTif` option. It should do it all for you

```shell
./scripts/generate_tif_from_hgt.sh --poly-shape $POLY --polyzoom 5 --elevation_tiles ./elevation_tiles --output ${AREA}.tif
```

All intermediates are VRT (no pixel written), so the only file created is the output itself.

`--overTif` only contributes **resolution**, never its own projection: the output always stays on the hgt
grid (`EPSG:4326`, same bounds, same `-10` nodata as without `--overTif`), just refined by an integer factor
so the finer pixels survive and both layers land on the exact same grid. A pixel comes from `overTif` where
it has data, from the hgt everywhere else.
That refined grid gets large (rhone-alpes at the IGN 5m resolution is ~29 gigapixel), so the output is
written tiled and `DEFLATE` compressed by default:

| flag | effect |
| --- | --- |
| `--refine <N>` | refine the hgt grid N times instead of the automatic factor (5 for IGN RGE ALTI 5m). `--refine 3` is ~3x smaller and still finer than a z14 tile |
| `--blur <METERS>` | width of the cross fade along the edge of the `overTif` coverage (default 1000, `--blur 0` disables). The two sources disagree there (different resolution, and EGM96 vs NGF-IGN69 vertical datum) and a hard edge shows up as a ridge in the hillshade. Costs ~3x on the final pass |
| `--compress <ALG>` | `LZW`, `DEFLATE` (default), `ZSTD`, ... |
| `--no-compress` | raw output, only if you have the disk for it |
| `--int16` | store elevation as `Int16` (1m resolution) instead of `Float32`. ~7x smaller, but quantizes the sub-meter precision of a 5m source like IGN RGE ALTI |

`GDAL_CACHEMAX=8192` in the environment raises the GDAL block cache (default 2048MB) if you want to throw
more RAM at it. Add overviews afterwards if you intend to open the result in QGIS:
```shell
gdaladdo -r average --config COMPRESS_OVERVIEW DEFLATE ${AREA}.tif
```

## # Terrain RGB tiles (single step, no intermediate tif)

`scripts/build_terrain_rgb.py` goes straight from the elevation sources to an mbtiles,
mapterhorn style: the max zoom is warped once per macrotile and every lower zoom is a
2x2 average of the tiles already produced, instead of one full pass over the dem per
zoom. No `${AREA}.tif` is built, so the whole `generate_tif_from_hgt.sh` step and its
disk cost disappear when you only want terrain tiles.

Sources are listed lowest priority first, and each one fades into the previous over
`--blur` metres so a better source overrides a coarser one without a step along the
edge of its coverage. `sources.json`:

```json
[
  {"name": "tilezen", "type": "valhalla", "path": "./elevation_tiles", "clamp_min": -10},
  {"name": "ignrge5", "type": "raster", "path": "work_france_ign/out/*.tif"}
]
```

| `type` | meaning |
| --- | --- |
| `valhalla` | the `.hgt` already downloaded for valhalla. Missing tiles are fetched with `valhalla_build_elevation` unless `"download": false`. 1 arc-second, and they carry bathymetry, hence `clamp_min` |
| `raster` | local file, glob or list of globs, in any CRS |
| `mapterhorn` | a [mapterhorn source-catalog](https://github.com/mapterhorn/mapterhorn/tree/main/source-catalog) source. `path` is a catalog name, a url to a `file_list.txt`, or a local one |

A `mapterhorn` source downloads what the area needs and extracts the archives (`zip`,
`tar`, `7z`, including the split `.7z.001/.002` deliveries, which need the `7z` cli).
The extracted folder is what gets checked first, so the archives can be deleted once
extracted without triggering a new download.

```json
{
  "name": "frrgealti1m", "type": "mapterhorn",
  "path": "https://raw.githubusercontent.com/mapterhorn/mapterhorn/main/source-catalog/frrgealti1metro/file_list.txt",
  "cache": "sources/frrgealti1m", "crs": "EPSG:2154", "allow_full_download": true
}
```

When the filenames carry their coordinates (`glo30`) only the intersecting tiles are
fetched. When they do not (the departement wide `.7z` of RGE ALTI) the area cannot be
known before downloading, so `allow_full_download` has to be set, and the rasters are
filtered on their real bounds once extracted. `crs` assigns a projection to deliveries
that ship without one, like the `.asc` of RGE ALTI.

```shell
python scripts/build_terrain_rgb.py --sources sources.json --poly-shape $POLY \
  --minzoom 5 --maxzoom 12 --round-digits 0 --encoding mapbox -f webp \
  --blur 1000 --tile-buffer 1 -j 16 -o ${OUTPUT_DIR}/${AREA}/${AREA}_terrain.mbtiles
```

`--encoding terrarium` writes the same tiles with the terrarium base/interval.
Keep `--round-digits 0` for 3d terrain, the quantization is a hillshade tradeoff.
`--max-round-digits` is what makes the quantization grow as the zoom drops, leave it
alone and every zoom quantizes like the max one.

`--tile-buffer 1` writes a ring of tiles around the poly, for 3d terrain renderers:
they backfill the 1px border of a dem tile from its neighbours, and a missing
neighbour is a seam at the edge of the covered area. The ring is a perimeter, so it
costs ~19% more tiles at the max zoom but a lot more at the low zooms (26 -> 52 tiles
at z9 for rhone-alpes), about 28% more bytes overall. Leave it at 0 for hillshades and
contours. The extra tiles needed to average a zoom from its 4 children are worked out
separately and never written.

## # Then build hillshades
```shell
./scripts/build_hillshades.sh --minzoom 5 --maxzoom 12 --round-digits 3 --max-round-digits 7  -o ${OUTPUT_DIR}/${AREA}/${AREA}_hillshade.mbtiles -f webp --poly-shape $POLY ${AREA}.tif
```

## # Then build contours
```shell
./scripts/build_contours.sh  --minzoom 11 --maxzoom 14 --poly-shape $POLY --output ${OUTPUT_DIR}/${AREA}/${AREA}_contours.mbtiles ${AREA}.tif
```

## # Cleanup contours mbtiles
This steps ensure we have the same tiles in the area mbtiles and hillshade/contours. It is important in AlpiMaps as we merge tiles from area and contours to draw contour lines in between the map style. 
```shell
python ./scripts/filter_tiles_from_other_mbtiles.py --sourcembtiles ${OUTPUT_DIR}/${AREA}/${AREA}.mbtiles ${OUTPUT_DIR}/${AREA}/${AREA}_contours.mbtiles
```
If you want you can also clear hillshade mbtiles (though it wont make much of a difference in size)
```shell
python ./scripts/filter_tiles_from_other_mbtiles.py --sourcembtiles ${OUTPUT_DIR}/${AREA}/${AREA}.mbtiles ${OUTPUT_DIR}/${AREA}/${AREA}_hillshade.mbtiles
```

## # build valhalla package

### Download the elevation tiles

`valhalla_build_tiles` bakes elevation into the graph during its `elevation` stage, so the tiles listed in
`additional_data.elevation` of `valhalla.json` (`./elevation_tiles`) must be downloaded **before** building the tiles.
The same folder is reused later by `generate_tif_from_hgt.sh`, so use `-d` (decompressed `.hgt`) here.

Compute the bounds of your poly, then download from Tilezen:
```shell
export BOUNDS=$(python ./scripts/get_shape_bounds_tile_envelope.py --poly-shape $POLY --minzoom 5 --maxzoom 5)
valhalla_build_elevation -v -d -b $BOUNDS -o ./elevation_tiles
```
Use the same `--minzoom/--maxzoom` value you intend to pass as `--polyzoom` to `generate_tif_from_hgt.sh` (5 here),
so both steps cover the same area and no tile gets downloaded twice.

If you build the valhalla graph for a parent area (see below), download the elevation for that parent area instead,
either with an explicit bbox (`-b '{w},{s},{e},{n}'`) or, once a graph already exists, straight from it:
```shell
valhalla_build_elevation -v -d -t -c valhalla.json
```

### Build the tiles

first build valhalla tiles if you didnt already. In my case i build valhalla tiles for the whole europe to ensure i have all tiles to calculate routes
between europe packages. So i only build valhalla tiles once . So here for my case i replace `$AREA` with `europe`
```shell
valhalla_build_tiles -c valhalla.json data/sources/${AREA//-/_}.osm.pbf
```
Then build valhalla "mbtiles package
```shell
python ./scripts/build_valhalla_package.py --id $AREA --poly $AREA.poly --polymaxzoom=11 valhalla_tiles ${OUTPUT_DIR}/${AREA}/${AREA}.vtiles
```
