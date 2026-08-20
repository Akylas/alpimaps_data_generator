#!/usr/bin/env bash
# Same recipe as L7_best, sweeping the global simplify tolerance down from 1.0.
# The landcover z11-13 override is scaled by the same ratio so polygons soften with lines.
set -e
cd "$(dirname "$0")/.."
JAR=planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar
COMMON="--area=rhone-alpes --languages= --force --compact-db --transportation-name-limit-merge --exclude_layers=route --nodemap-type=sparsearray --polygon=rhone-alpes.poly --max-point-buffer=4 --output-layerstats"
MAXZ="--simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25"

run() { # name globaltol landcovertol
  echo "=== $1  tol=$2 landcover=$3 ==="
  java -Xmx32g -jar $JAR $COMMON $MAXZ \
    --simplify-tolerance=$2 \
    --landcover_tolerance_z11_13=$3 \
    --landcover_drop_redundant_subclass=true --landcover_merge_maxzoom=true \
    --mbtiles=bench/$1.mbtiles > bench/$1.log 2>&1
  ls -l bench/$1.mbtiles
}

run T085 0.85 1.3
run T070 0.70 1.05
run T060 0.60 0.9
echo "=== done ==="
ls -l bench/T0*.mbtiles
