#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/.."
JAR=planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar
COMMON="--area=rhone-alpes --languages= --force --compact-db --transportation-name-limit-merge --exclude_layers=route --nodemap-type=sparsearray --polygon=rhone-alpes.poly --max-point-buffer=4 --output-layerstats"
SAFE="--simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 --simplify-tolerance=1.0"
run() { name=$1; shift; echo "=== $name ==="; java -Xmx32g -jar $JAR $COMMON $SAFE --mbtiles=bench/$name.mbtiles "$@" > bench/$name.log 2>&1; ls -l bench/$name.mbtiles; }

# subclass drop, now that the merge grouping falls back to class
run L6_subclass_fixed --landcover_drop_redundant_subclass=true
# gentler z11-13 tolerance than the 2.5 that collapsed 22% of z11 polygons
run L5_tol15 --landcover_tolerance_z11_13=1.5
# best combination: gentle tolerance + working subclass drop + z14 merge
run L7_best --landcover_tolerance_z11_13=1.5 --landcover_drop_redundant_subclass=true --landcover_merge_maxzoom=true
echo "=== done ==="
ls -l bench/L*.mbtiles
