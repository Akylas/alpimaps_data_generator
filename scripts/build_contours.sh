#!/bin/bash

set -e


 usage() {
	echo "usage: $0 <source> [--output=output-filename] [--polyshape=polyshape] [--format=format] [--minzoom=minzoom] [--maxzoom=maxzoom]";
	exit 1;
}

source=""
output=""
minzoom="11"
maxzoom="14"
polyshape=""
polyzoom="11"
bounds=""
name="Contours-10m"
layer="contour"
maxrounddigits=100
rounddigits=4

while [ $# -gt 0 ]; do
  case "$1" in
    -o|--output*)
      output="$2"
      ;;
    -s|--source*)
      source="$2"
      ;;
    -s|--bounds*)
      bounds="$2"
      ;;
    -p|--poly-shape*)
      polyshape="$2"
      ;;
    --minzoom*)
      minzoom="$2"
      ;;
    --maxzoom*)
      maxzoom="$2"
      ;;
    --name*)
      name="$2"
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

#Contours building
sourceName=$(echo "$source" | cut -f 1 -d '.')

if([[ -n $polyshape ]]); then
  echo "python ./scripts/get_shape_bounds_tile_envelope.py --poly-shape $polyshape --maxzoom $maxzoom --minzoom $minzoom"
  bounds=$(python ./scripts/get_shape_bounds_tile_envelope.py --poly-shape $polyshape --maxzoom $maxzoom --minzoom $minzoom
)
fi

if ([ "$output" = "" ]); then
  output="$sourceName-contours-10m.mbtiles"
fi

echo "source $source"
echo "sourceName $sourceName"
echo "bounds $bounds"
echo "name $name"

if( [[ -n $bounds ]]); then
  IFS=',' read -ra split_bounds <<< "$bounds"
  echo "gdal_translate -projwin_srs EPSG:4326 -projwin ${split_bounds[0]} ${split_bounds[3]} ${split_bounds[2]} ${split_bounds[1]} $source ${sourceName}_extract.tif"
  gdal_translate -projwin_srs EPSG:4326 -projwin ${split_bounds[0]} ${split_bounds[3]} ${split_bounds[2]} ${split_bounds[1]} $source ${sourceName}_extract.tif
  source="${sourceName}_extract.tif"
fi


echo "gdal_contour -i 10 -a ele $source $name-contours-10m.gpkg"
gdal_contour -i 10 -a ele $source $name-contours-10m.gpkg
echo "ogr2ogr -t_srs EPSG:4326 $name-contours-4326-10m.gpkg $name-contours-10m.gpkg"
ogr2ogr -t_srs EPSG:4326 $name-contours-4326-10m.gpkg $name-contours-10m.gpkg
rm $name-contours-10m.gpkg

echo "ogr2ogr -dialect sqlite -sql ... /vsigzip/$name-contours-10m.geojson $name-contours-4326-10m.gpkg"

ogr2ogr -dialect sqlite -sql "
SELECT
  ele,
  CASE
    WHEN ele % 1000 = 0 THEN 1000
    WHEN ele % 500 = 0 THEN 500
    WHEN ele % 250 = 0 THEN 250
    WHEN ele % 200 = 0 THEN 200
    WHEN ele % 100 = 0 THEN 100
    WHEN ele % 50 = 0 THEN 50
    WHEN ele % 20 = 0 THEN 20
    ELSE 10
  END AS div,
  geom
FROM
  contour
" /vsigzip/$name-contours-10m.geojson $name-contours-4326-10m.gpkg
rm $name-contours-4326-10m.gpkg
mv $name-contours-10m.geojson $name-contours-10m.geojson.gz

ARGS="-Z$minzoom -z$maxzoom --no-tile-stats --read-parallel --force --name=\"$name\" -pk -pf -S 3 --layer=$layer -o $output $name-contours-10m.geojson.gz"

if( [[ -n $bounds ]]); then
  ARGS="$ARGS --clip-bounding-box=$bounds"
fi

TIPPECANOE_FILTER='{"*": ["any",["all",["<=", "$zoom", 6],[">=", "div", 1000]],["all",[">=", "$zoom", 7],["<=", "$zoom", 7],[">=", "div", 500]],["all",[">=", "$zoom", 8],["<=", "$zoom", 9],["!=", "div", 500],[">=", "div", 200]],["all",[">=", "$zoom", 10],["<=", "$zoom", 11],[">=", "div", 100]],["all",["==", "$zoom", 12],[">=", "div", 100]],["all",["==", "$zoom", 13],[">=", "div", 50]],["all",[">=", "$zoom", 14]]]}'

echo "tippecanoe $ARGS -j '$TIPPECANOE_FILTER'"
tippecanoe $ARGS -j  \
  '{"*": ["any",["all",["<=", "$zoom", 6],[">=", "div", 1000]],["all",[">=", "$zoom", 7],["<=", "$zoom", 7],[">=", "div", 500]],["all",[">=", "$zoom", 8],["<=", "$zoom", 9],["!=", "div", 500],[">=", "div", 200]],["all",[">=", "$zoom", 10],["<=", "$zoom", 11],[">=", "div", 100]],["all",["==", "$zoom", 12],[">=", "div", 100]],["all",["==", "$zoom", 13],[">=", "div", 50]],["all",[">=", "$zoom", 14]]]}'


if ([[ -n $polyshape ]]); then
  python scripts/generate_poly_tilemask.py --poly $polyshape --maxzoom $polyzoom ${output} 
fi

rm $name-contours-10m.geojson.gz
rm ${sourceName}_extract.tif
