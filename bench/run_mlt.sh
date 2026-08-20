#!/usr/bin/env bash
# MLT flag sweep, on top of the exact flag set already in use.
set -e
cd "$(dirname "$0")/.."
JAR=planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar
COMMON="--area=rhone-alpes --languages= --force --compact-db --transportation-name-limit-merge \
--exclude_layers=route --nodemap-type=sparsearray --polygon=rhone-alpes.poly --max-point-buffer=4 \
--transportation_z13_paths --parallel-tmp-io \
--simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 \
--landcover_tolerance_z11_13=1.05 --landcover_drop_redundant_subclass=true --landcover_merge_maxzoom=true"

run() {
  name=$1; shift
  echo "=== $name ==="
  java -Xmx32g -jar $JAR $COMMON --mbtiles=bench/$name.mbtiles "$@" > bench/$name.log 2>&1
  ls -l bench/$name.mbtiles
}

run M0_mvt
run M1_mlt_shareddict   --tile-format=mlt --mlt-shared-dict
run M2_mlt_advanced     --tile-format=mlt --mlt-shared-dict --mlt-advanced
run M3_mlt_reorder      --tile-format=mlt --mlt-shared-dict --mlt-advanced --mlt-reorder-features
echo "=== done ==="
ls -l bench/M*.mbtiles
