#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/.."

JAR=planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar
# nodemap-type=sparsearray instead of array: identical output, but array mmaps ~67G for this
# extract and thrashes on an external volume. Purely a build-speed choice.
COMMON="--area=rhone-alpes --languages= --force --compact-db --transportation-name-limit-merge --nodemap-type=sparsearray --polygon=rhone-alpes.poly --max-point-buffer=4 --output-layerstats"

run() {
  name=$1; shift
  echo "=== $name ==="
  java -Xmx32g -jar $JAR $COMMON --mbtiles=bench/$name.mbtiles "$@" > bench/$name.log 2>&1
  ls -l bench/$name.mbtiles
}

# v2: new jar only (name_int no longer duplicates name), same flags as baseline
# already built - skip
# run v2_nameint --exclude_layers=route

# v3: + max-zoom simplification raised off sub-pixel defaults (z14 = 71% of bytes)
run v3_simplify14 --exclude_layers=route \
  --simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25

# v4: + below-max-zoom simplification raised
run v4_simplify_all --exclude_layers=route \
  --simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 \
  --simplify-tolerance=1.0 --min-feature-size=4

# v5: + drop housenumber
run v5_no_housenumber --exclude_layers=route,housenumber \
  --simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 \
  --simplify-tolerance=1.0 --min-feature-size=4

# v6: + drop building/building_name too (app decision, shown for reference)
run v6_no_building --exclude_layers=route,housenumber,building,building_name \
  --simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 \
  --simplify-tolerance=1.0 --min-feature-size=4

echo "=== done ==="
ls -l bench/*.mbtiles
