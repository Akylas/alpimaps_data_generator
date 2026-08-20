#!/usr/bin/env bash
# Routes, keeping name/extent/symbol. Everything here stays renderable from the style.
set -e
cd "$(dirname "$0")/.."

JAR=planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar
COMMON="--area=rhone-alpes --languages= --force --compact-db --transportation-name-limit-merge --only_layers=route --nodemap-type=sparsearray --polygon=rhone-alpes.poly --max-point-buffer=4 --output-layerstats"

# the exact simplification the basemap ladder used (v4_simplify_all)
BASEMAP_SIMPLIFY="--simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 --simplify-tolerance=1.0 --min-feature-size=4"

run() {
  name=$1; shift
  echo "=== $name ==="
  java -Xmx32g -jar $JAR $COMMON --mbtiles=bench/$name.mbtiles "$@" > bench/$name.log 2>&1
  ls -l bench/$name.mbtiles
}

# k1: only fix the tolerance mismatch with the transportation layer. No attrs touched.
run k1_roadtol --route_road_tolerance=true

# k2: + extent at 2 decimals (~1.1km, still far finer than a route bbox needs)
run k2_extent2 --route_road_tolerance=true --route_extent_digits=2

# k3: + osmc:symbol as an integer id with a sidecar lookup table
run k3_symbolid --route_road_tolerance=true --route_extent_digits=2 \
  --route_symbol_id=true --route_symbol_table=bench/k3_symbols.json

# k4: + the same simplification drop the basemap got, so routes and roads match
run k4_basemap_simplify --route_road_tolerance=true --route_extent_digits=2 \
  --route_symbol_id=true --route_symbol_table=bench/k4_symbols.json \
  $BASEMAP_SIMPLIFY

# k5: basemap simplification but WITHOUT the road-tolerance fix, to separate the two effects
run k5_simplify_only --route_extent_digits=2 $BASEMAP_SIMPLIFY

echo "=== done ==="
ls -l bench/k*.mbtiles
