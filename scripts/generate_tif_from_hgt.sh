#!/bin/bash

set -e
# set -o xtrace

usage() {
	echo "usage: $0 <source> [--elevation_tiles=elevation_tiles_folder] [--output=output-filename] [--poly-shape=polyshape] [--bounds=bounds] [--polyzoom=polyzoom]";
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

output=""
polyzoom="14"
polyshape=""
bounds=""
elevation_tiles=""

while [ $# -gt 0 ]; do
  case "$1" in
    -o|--output*)
      output="$2"
      ;;
    -o|--elevation_tiles*)
      elevation_tiles="$2"
      ;;
    -s|--bounds*)
      bounds="$2"
      ;;
    -p|--poly-shape*)
      polyshape="$2"
      ;;
    -z|--polyzoom*)
      polyzoom="$2"
      ;;
    *)
  esac
  shift
done


[ "$elevation_tiles" ] && ([ "$polyshape" ] || [ "$bounds" ]) || usage


echo "output $output"
echo "elevation_tiles $elevation_tiles"
echo "polyshape $polyshape"
echo "polyzoom $polyzoom"
if [ ! -z "${polyshape}" ]; then
  echo "python ./scripts/get_shape_bounds_tile_envelope.py --poly-shape $polyshape --maxzoom $polyzoom --minzoom $polyzoom"
  bounds=$(python ./scripts/get_shape_bounds_tile_envelope.py --poly-shape $polyshape --maxzoom $polyzoom --minzoom $polyzoom
)
fi

echo "bounds $bounds"
echo "bounds2 ${bounds//,/ }"


echo "valhalla_build_elevation -b $bounds -o ${elevation_tiles}"
valhalla_build_elevation -d -b $bounds -o ${elevation_tiles}

echo "gdalbuildvrt -te ${bounds//,/ } -hidenodata  elevation_tiles.vrt elevation_tiles/**/*.hgt"
gdalbuildvrt -te ${bounds//,/ } -hidenodata  elevation_tiles.vrt elevation_tiles/**/*.hgt
echo "gdal_translate -co compress=lzw -of GTiff elevation_tiles.vrt elevation_tiles.tif  -co BIGTIFF=YES"
gdal_translate -co compress=lzw -of GTiff elevation_tiles.vrt elevation_tiles.tif  -co BIGTIFF=YES
echo "gdal_calc.py  --overwrite --co \"BIGTIFF=YES\" --co=\COMPRESS=LZW\" --type=Float32 -A  elevation_tiles.tif --outfile=elevation_tiles_0.tif --calc=\"((A+10)*(A+10>0))-10\" --NoDataValue=-10"
gdal_calc.py --overwrite --co "BIGTIFF=YES" --co="COMPRESS=LZW" --type=Float32 -A  elevation_tiles.tif --outfile=elevation_tiles_0.tif --calc="((A+10)*(A+10>0))-10" --NoDataValue=-10
echo "gdalwarp -tr 25 25 -ot Float32 elevation_tiles_0.tif -s_srs EPSG:4326 -t_srs EPSG:2154  -of GTiff $output  -co BIGTIFF=YES"
gdalwarp -tr 25 25 -ot Float32 elevation_tiles_0.tif -s_srs EPSG:4326 -t_srs EPSG:2154  -of GTiff $output  -co BIGTIFF=YES
rm elevation_tiles.vrt
rm elevation_tiles_0.tif
rm elevation_tiles.tif
