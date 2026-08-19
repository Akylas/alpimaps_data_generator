#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/.."

JAR=planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar
COMMON="--area=rhone-alpes --languages= --force --compact-db --transportation-name-limit-merge --only_layers=route --nodemap-type=sparsearray --polygon=rhone-alpes.poly --max-point-buffer=4 --output-layerstats"

run() {
  name=$1; shift
  echo "=== $name ==="
  java -Xmx32g -jar $JAR $COMMON --mbtiles=bench/$name.mbtiles "$@" > bench/$name.log 2>&1
  ls -l bench/$name.mbtiles
}

# r_base already built with the pre-change jar; rebuild it here so every file comes
# from the same jar and only the flags differ.
run r_v0_base

# drop the per-tile relation bbox string
run r_v1_noextent --route_drop_extent=true

# tiles carry only osmid/class/network; the rest moves to a side lookup
run r_v2_slim --route_slim_attrs=true

# + cull merged segments below minLength at z<max (the currently-dead minLength)
run r_v3_minlength --route_slim_attrs=true --route_min_length=true

# + the same geometry simplification that worked on the basemap
run r_v4_simplify --route_slim_attrs=true --route_min_length=true \
  --simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25 \
  --simplify-tolerance=1.0 --min-feature-size=4

echo "=== done ==="
ls -l bench/r_*.mbtiles
