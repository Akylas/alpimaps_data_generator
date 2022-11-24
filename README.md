
## # Dependencies

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
java -jar ./planetiler-dist/target/planetiler-dist-0.5-SNAPSHOT-with-deps.jar  --only-download --area=$AREA
valhalla_build_config --mjolnir-tile-dir ${PWD}/valhalla_tiles --mjolnir-tile-extract ${PWD}/valhalla_tiles.tar --mjolnir-timezone ${PWD}/valhalla_tiles/timezones.sqlite --mjolnir-admin ${PWD}/valhalla_tiles/admins.sqlite --additional-data-elevation ${PWD}/elevation_tiles > conf/valhalla.json 
```

## # First generate the mbtiles with Planetiler

```shell
java -Xmx32g -jar ./planetiler-dist/target/planetiler-dist-0.5-SNAPSHOT-with-deps.jar  --download --area=europe --languages="" --force --compact-db --transportation-name-limit-merge -only_layers=route --nodemap-type=array --mbtiles=${AREA}.mbtiles --polygon=$POLY
```
Now generate the tif of the area you want. It is best to use polyzoom as the min zoom you want for hillshades / contours. This ensure you wont get half filled tiles. Though the process will be slower and the tif bigger

```shell
./scripts/generate_tif_from_hgt.sh --poly-shape $POLY --polyzoom 5 --elevation_tiles ./elevation_tiles --output ${AREA}.tif
```

## # Then build hillshades
```shell
./scripts/build_hillshades.sh --minzoom 5 --maxzoom 12 -r 3 --max-round-digits 7  -o ${AREA}_hillshade.mbtiles -f webp --poly-shape $POLY ${AREA}.tif
rio rgbify -b -10000 -i 0.1 --max-z 12 --min-z 5 --format webp -j 20 --poly-shape $POLY ${AREA}.tif ${AREA}_hillshade.mbtiles
```

## # Then build contours
```shell
./scripts/build_contours.sh --poly-shape $POLY --output ${AREA}_contours.mbtiles ${AREA}.tif
```

## # Cleanup contours mbtiles
```shell
python ./scripts/filter_tiles_from_other_mbtiles.py --sourcembtiles /home/mguillon/dev/planetiler/data/${AREA}.mbtiles ${AREA}_contours.mbtiles
```
If you want you can also clear hillshade mbtiles (though it wont make much of a difference in size)
```shell
python ./scripts/filter_tiles_from_other_mbtiles.py --sourcembtiles /home/mguillon/dev/planetiler/data/${AREA}.mbtiles ${AREA}_hillshade.mbtiles
```

## # build valhalla package

Now here valhalla expects the pbf extract only of the country we use.
But if you built the country mbtiles using "bigger" area to have a full mbtiles you wont have the right extract.
In that case you need to download it first

first build valhalla tiles if you didnt already. In my case i build valhalla tiles for the whole europe to ensure i have all tiles to calculate routes
between europe packages. So i only build valhalla tiles once
```shell
valhalla_build_tiles -c valhalla.json $AREA-latest.osm.pbf
```
Then build valhalla "mbtiles package
```shell
python3 mobile-sdk-scripts/scripts/build_valhalla_packages.py data/packages-carto.json.template valhalla_tiles ./
```
