#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/.."
JAR=planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar
COMMON="--area=rhone-alpes --languages= --force --compact-db --transportation-name-limit-merge --exclude_layers=route --nodemap-type=sparsearray --polygon=rhone-alpes.poly --max-point-buffer=4 --output-layerstats"
# the safe geometry baseline agreed on: vertex simplification only, no feature deletion
SAFE="--simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 --simplify-tolerance=1.0"
run() { name=$1; shift; echo "=== $name ==="; java -Xmx32g -jar $JAR $COMMON --mbtiles=bench/$name.mbtiles "$@" > bench/$name.log 2>&1; ls -l bench/$name.mbtiles; }

run L0_control  $SAFE
run L1_merge14  $SAFE --landcover_merge_maxzoom=true
# 2.5 = the factor Landcover asks for via the dead setPixelToleranceFactor, applied to the
# --simplify-tolerance=1.0 that SAFE sets. 0.5 would be *less* simplification than the default.
run L2_tol1113  $SAFE --landcover_tolerance_z11_13=2.5
run L3_subclass $SAFE --landcover_drop_redundant_subclass=true
run L4_all      $SAFE --landcover_merge_maxzoom=true --landcover_tolerance_z11_13=2.5 --landcover_drop_redundant_subclass=true
echo "=== done ==="
ls -l bench/L*.mbtiles
