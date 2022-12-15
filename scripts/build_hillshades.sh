#!/usr/bin/bash

set -e
# set -o xtrace

 usage(){
	echo "usage: $0 <source> [--output=output-filename] [--polyshape=polyshape] [--format=format] [--minzoom=minzoom] [--maxzoom=maxzoom]";
	exit 1;
}

max() {
  local res=$1
  if [[ $1 -gt $2 ]]; then
    res=$2
  fi
  echo "$res"
}
min() {
  local res=$2
  if [[ $1 -lt $2 ]]; then
    res=$1
  fi
  echo "$res"
}

source=""
output=""
minzoom="5"
maxzoom="12"
polyshape=""
polyzoom="11"
format="webp"
maxrounddigits=100
rounddigits=4

while [ $# -gt 0 ]; do
  case "$1" in
    -o|--output*)
      output="$2"
      ;;
    -f|--format*)
      format="$2"
      ;;
    -s|--source*)
      source="$2"
      ;;
    -p|--poly-shape*)
      polyshape="$2"
      ;;
    --minzoom*)
      minzoom="$2"
      ;;
    -r|--round-digits*)
      rounddigits=$2
      ;;
    --max-round-digits*)
      maxrounddigits=$2
      ;;
    --maxzoom*)
      maxzoom="$2"
      ;;
    --polyzoom*)
      polyzoom="$2"
      ;;
    *)
    source="$1"
  esac
  shift
done
[ "$source" ] || usage


sourceNameWithExt=${source##*/}
sourceName=${sourceNameWithExt%.*}

echo "source $source"
echo "format $format"
echo "minzoom $minzoom"
echo "maxzoom $maxzoom"
echo "rounddigits $rounddigits"
echo "maxrounddigits $maxrounddigits"

#Generate MBTiles
digits=""
for ((n = $rounddigits ; n <= $(($maxzoom-$minzoom+$rounddigits)) ; n++)); do
    z=$(($maxzoom-n+$rounddigits))
    actualn=$(min $n $maxrounddigits)
    digits="$digits $actualn"
    args="--format ${format} -j16 -b -10000 -i 0.1 --max-z ${z} --min-z ${z} --round-digits ${actualn}"
    if [[ -n $polyshape ]]; then
        args="$args --poly-shape ${polyshape}"
    fi
    echo "rio rgbify $args  ${source}  ${sourceName}_${z}_rgb_$format.mbtiles"
    rio rgbify $args ${source} ${sourceName}_${z}_rgb_$format.mbtiles
done


sqlitecommand="
--PRAGMA journal_mode=PERSIST;
--PRAGMA page_size=80000;
--PRAGMA synchronous=OFF;
UPDATE metadata  SET value='${digits}' WHERE name='round-digits';
UPDATE metadata  SET value=${minzoom} WHERE name='minzoom';
UPDATE metadata  SET value=${maxzoom} WHERE name='maxzoom';"
for ((n = $minzoom ; n < $maxzoom ; n++)); do
    sqlitecommand="${sqlitecommand}
ATTACH DATABASE '${sourceName}_${n}_rgb_${format}.mbtiles' AS m${n};
REPLACE INTO tiles_shallow SELECT * FROM m${n}.tiles_shallow;
REPLACE INTO tiles_data SELECT * FROM m${n}.tiles_data;"
done

if [ "$output" = "" ]; then
    output="${sourceName}_rgb_${format}.mbtiles"
fi
cp  ${sourceName}_${maxzoom}_rgb_$format.mbtiles ${output}
echo "sqlite3  ${output} \"${sqlitecommand}\""
sqlite3  ${output} "${sqlitecommand}"

if ([[ -n $polyshape ]]); then
  python scripts/generate_poly_tilemask.py --poly $polyshape --maxzoom $polyzoom ${output} 
fi

for ((n = $rounddigits ; n <= $(($maxzoom-$minzoom+$rounddigits)) ; n++)); do
    z=$(($maxzoom-n+$rounddigits))
    rm ${sourceName}_${z}_rgb_${format}.mbtiles
done

