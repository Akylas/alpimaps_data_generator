
### Introduction

That repo explains how to generate data to be used with [AlpiMaps](https://github.com/Akylas/alpimaps).
It can also be used to generate mbtiles to be used with other projects like `tileserver-gl`, `qgis` ...

### macos

```shell
brew install aria2 gdal autoconf automake zmq czmq
pip3 install gdal

```

### ubuntu

```shell
sudo add-apt-repository ppa:ubuntugis/ubuntugis-unstable
sudo apt install -y aria2 gdal-bin autoconf automake pkg-config libtool make gcc g++ lcov cmake make libtool pkg-config g++ gcc curl unzip jq lcov protobuf-compiler vim-common locales libcurl4-openssl-dev zlib1g-dev liblz4-dev libprotobuf-dev
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
java -Xmx32g -jar $PLANETILER_JAR  --download --area=${AREA} --languages="" --force --compact-db --transportation-name-limit-merge -exclude_layers=route --nodemap-type=array --mbtiles=${OUTPUT_DIR}/${AREA}/${AREA}.mbtiles --polygon=$POLY --max-point-buffer=4
```
Or build using a "parent" area. For example i will always use europe as i mostly build europe countries

```shell
java -Xmx32g -jar $PLANETILER_JAR  --download --area=europe --languages="" --force --compact-db --transportation-name-limit-merge -exclude_layers=route --nodemap-type=array --mbtiles=${OUTPUT_DIR}/${AREA}/${AREA}.mbtiles --polygon=$POLY --max-point-buffer=4
```

If you want to generate low level world map:
```shell
java -Xmx32g -jar $PLANETILER_JAR  --download --area=planet --languages=fr,en --force --transportation-name-limit-merge --compact-db --only_layers=place,park,boundary,mountain_peak,transportation,transportation_name,water,waterway,water_name,landcover,landcover_name,landuse --maxzoom=7 --nodemap-type=array --mbtiles=${OUTPUT_DIR}/world.mbtiles --max-point-buffer=4


## # Generate routes mbtiles

```shell
java -Xmx32g -jar $PLANETILER_JAR  --download --area=${AREA} --languages="" --force --compact-db --transportation-name-limit-merge -only_layers=route --nodemap-type=array --mbtiles=${OUTPUT_DIR}/${AREA}/${AREA}_routes.mbtiles --polygon=$POLY --max-point-buffer=4
```

## # Generate area tif

Now generate the tif of the area you want. It is best to use polyzoom as the min zoom you want for hillshades / contours. This ensure you wont get half filled tiles. Though the process will be slower and the tif bigger
We use a small polyzoom (5) to avoid half filled tiles. You can raise it for faster build but you ll have half filled tiles in hillshades or contours (mostly hillshade as contours is starting at zoom 11 here)
If you want to have more defined tif over (that you would get from another source) you can use the `--overTif` option. It should do it all for you

```shell
./scripts/generate_tif_from_hgt.sh --poly-shape $POLY --polyzoom 5 --elevation_tiles ./elevation_tiles --output ${AREA}.tif
```


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
first build valhalla tiles if you didnt already. In my case i build valhalla tiles for the whole europe to ensure i have all tiles to calculate routes
between europe packages. So i only build valhalla tiles once . So here for my case i replace `$AREA` with `europe`
```shell
valhalla_build_tiles -c valhalla.json data/sources/$AREA.osm.pbf
```
Then build valhalla "mbtiles package
```shell
python ./scripts/build_valhalla_package.py --id $AREA --poly $AREA.poly --polymaxzoom=11 valhalla_tiles ${OUTPUT_DIR}/${AREA}/${AREA}.vtiles
```
