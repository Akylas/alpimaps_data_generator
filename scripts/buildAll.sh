#!/bin/bash

set -e

area=""
languages=""
source=""
output=""
minzoom="11"
maxzoom="14"
polyshape=""
bounds=""

#build mbtiles
java -Xmx32g -jar ./planetiler-dist/target/planetiler-dist-0.5-SNAPSHOT-with-deps.jar  --download --area=france --languages="" --force --compact-db --transportation-name-limit-merge --exclude_layers=route --mbtiles=data/france.mbtiles

#build route mbtiles
java -Xmx32g -jar ./planetiler-dist/target/planetiler-dist-0.5-SNAPSHOT-with-deps.jar  --download --area=france --languages="" --force --compact-db --transportation-name-limit-merge --only_layers=route --mbtiles=data/france_routes.mbtiles

# build hillshading
sh scripts/build_hillshades.sh --minzoom 5 --maxzoom 12 -r 3 --max-round-digits 7  -o france_terrain.mbtiles -f webp --poly-shape /Volumes/data/../dev/planetiler/planetiler/data/sources/france.poly  /Volumes/data/../dev/openmaptiles/test.tif

# build contours
sh ./scripts/build_contours.sh  --output france_contours.mbtiles --poly-shape /Volumes/data/../dev/planetiler/planetiler/data/sources/france.poly production_data/france/terrain_0.tif

#build valhalla