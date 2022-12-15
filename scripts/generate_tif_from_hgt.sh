#!/bin/bash

set -e
set -x #echo on

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
compressArg=""
compressArgPy=""
overTif=""

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
    -c|--compress*)
      compressArg="-co compress=lzw"
      compressArgPy="--co=\"COMPRESS=LZW\""
      ;;
    -o|--overTif*)
      overTif="$2"
      ;;
    *)
  esac
  shift
done

coArgs="-co BIGTIFF=YES"
coArgsPy="--co \"BIGTIFF=YES\""


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



valhalla_build_elevation -v -d -b $bounds -o ${elevation_tiles}

gdalbuildvrt -te ${bounds//,/ }  elevation_tiles.vrt elevation_tiles/**/*.hgt


srs=""

if [ ! -z "${overTif}" ]; then
  tempOverTif="tempOverTif.tif"
  info=$(gdalinfo ${overTif})
  pixelSize=$(gdalinfo ${overTif} | sed  -rn 's/Pixel Size = \((.*?),-(.*?)\)/\1 \2/pg')
  overTifSrs=$(gdalinfo ${overTif} | sed -rn  's/\s*ID\[\"EPSG\",([0-9]+)\]\]$/\1/pg')
  #in this case we need to warp elevation_tiles into over tif srs
  gdalwarp  $coArgs -tr $pixelSize -ot Float32 elevation_tiles.vrt -t_srs EPSG:$overTifSrs  -of GTiff elevation_tiles.tif

  projwin="$(gdalinfo elevation_tiles.tif | sed -n -E 's/Upper Left\s*\(([^\)]+),([^\)]+)\)(.*)/\1 \2/p') $(gdalinfo elevation_tiles.tif | sed -n -E 's/Lower Right\s*\(([^\)]+),([^\)]+)\)(.*)/\1 \2/p')"
  #we also resize overTif to the size of elevation_tiles.tif to ensure 
  #the result does not have any unwanted offset
  gdal_translate $coArgs ${overTif}  ${tempOverTif} -projwin $projwin

  #we compute NoDataValue 
  gdal_calc.py --overwrite $coArgsPy $compressArgPy --type=Float32 -A  elevation_tiles.tif --outfile=elevation_tiles_0.tif --calc="((A+10)*(A+10>0))-10" --NoDataValue=-10

  #we finally merge both with overTif on top
  gdalbuildvrt merged.vrt elevation_tiles_0.tif ${tempOverTif}
  gdal_translate merged.vrt $output
  rm elevation_tiles_0.tif
  rm merged.vrt
  rm ${tempOverTif}
else
  gdal_translate -of GTiff elevation_tiles.vrt elevation_tiles.tif $coArgs  $compressArg
  #we compute NoDataValue 
  gdal_calc.py --overwrite $coArgsPy $compressArgPy --type=Float32 -A  elevation_tiles.tif --outfile=$output --calc="((A+10)*(A+10>0))-10" --NoDataValue=-10

fi

rm elevation_tiles.vrt
rm elevation_tiles.tif
