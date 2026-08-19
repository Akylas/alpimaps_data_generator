#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/.."
JAR=planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar
COMMON="--area=rhone-alpes --languages= --force --compact-db --transportation-name-limit-merge --exclude_layers=route --nodemap-type=sparsearray --polygon=rhone-alpes.poly --max-point-buffer=4 --output-layerstats"
MAXZ="--simplify-tolerance-at-max-zoom=0.25 --min-feature-size-at-max-zoom=0.25"
run() { name=$1; shift; echo "=== $name ==="; java -Xmx32g -jar $JAR $COMMON --mbtiles=bench/$name.mbtiles "$@" > bench/$name.log 2>&1; ls -l bench/$name.mbtiles; }

# v3 + only the vertex simplification, no feature deletion
run s2_tol_only $MAXZ --simplify-tolerance=1.0
# v3 + only the feature-deletion threshold
run s3_minsize_only $MAXZ --min-feature-size=4
