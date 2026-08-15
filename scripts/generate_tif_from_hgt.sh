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
compressAlg="DEFLATE"
outType="Float32"
overTif=""
refine="auto"
blurMeters="1000"

while [ $# -gt 0 ]; do
  case "$1" in
    --output*)
      output="$2"
      ;;
    --elevation_tiles*)
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
    --no-compress)
      compressAlg="NONE"
      ;;
    -c|--compress*)
      compressAlg="$2"
      ;;
    --int16*)
      outType="Int16"
      ;;
    --refine*)
      refine="$2"
      ;;
    --blur*)
      blurMeters="$2"
      ;;
    --overTif*)
      overTif="$2"
      ;;
    *)
  esac
  shift
done

predictor="3"
[ "$outType" = "Int16" ] && predictor="2"

#the warp is executed lazily when the final gdal_translate reads the warped vrt,
#so the threading has to be set on every reader, not just on gdalwarp itself
gdalCacheMax="${GDAL_CACHEMAX:-2048}"
threadArgs="--config GDAL_NUM_THREADS ALL_CPUS --config GDAL_CACHEMAX $gdalCacheMax"

coArgs="-co BIGTIFF=YES -co TILED=YES -co NUM_THREADS=ALL_CPUS"
coArgsPy="--co=BIGTIFF=YES --co=TILED=YES --co=NUM_THREADS=ALL_CPUS"
if [ "$compressAlg" != "NONE" ]; then
  coArgs="$coArgs -co COMPRESS=$compressAlg -co PREDICTOR=$predictor"
  coArgsPy="$coArgsPy --co=COMPRESS=$compressAlg --co=PREDICTOR=$predictor"
fi


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
  tempOverVrt="tempOverTif.vrt"
  tempOverGrayVrt="tempOverTif_gray.vrt"
  overProbeVrt="tempOverTifProbe.vrt"
  maskCoarseVrt="overTifMask_coarse.vrt"
  maskAlphaVrt="overTifMask_alpha.vrt"
  maskFineVrt="overTifMask_fine.vrt"
  warpVrt="elevation_tiles_warped.vrt"
  warpGrayVrt="elevation_tiles_warped_gray.vrt"

  #overTif only brings resolution, never its own projection: the output stays on
  #the hgt grid (EPSG:4326) so that it matches the one produced without --overTif.
  #everything stays a VRT (no pixel written) until the very last gdal_translate,
  #otherwise each intermediate is a full size uncompressed raster (>100GB)
  hgtInfo=$(gdalinfo elevation_tiles.vrt)
  hgtRes=$(echo "${hgtInfo}" | sed -n -E 's/^Pixel Size = \(([^,]+),-?([^\)]+)\)/\1/p')
  hgtUl=$(echo "${hgtInfo}" | sed -n -E 's/^Upper Left[[:space:]]*\([[:space:]]*([^,]+),[[:space:]]*([^\)]+)\).*/\1 \2/p')
  hgtLr=$(echo "${hgtInfo}" | sed -n -E 's/^Lower Right[[:space:]]*\([[:space:]]*([^,]+),[[:space:]]*([^\)]+)\).*/\1 \2/p')
  read -r ulx uly <<< "$hgtUl"
  read -r lrx lry <<< "$hgtLr"

  overInfo=$(gdalinfo ${overTif})
  overNodata=$(echo "${overInfo}" | sed -n -E 's/^[[:space:]]*NoData Value=(.*)$/\1/p')

  #refine the hgt grid by an integer factor, so the finer overTif pixels survive
  #and both layers land on the exact same grid (no resampling offset at all)
  gdalwarp -q -of VRT -overwrite -t_srs EPSG:4326 -r bilinear ${overTif} ${overProbeVrt}
  overRes=$(gdalinfo ${overProbeVrt} | sed -n -E 's/^Pixel Size = \(([^,]+),-?([^\)]+)\)/\1/p')
  rm ${overProbeVrt}
  if [ "$refine" = "auto" ]; then
    refine=$(awk -v a="$hgtRes" -v b="$overRes" 'BEGIN { f = a / b; printf "%d", (f > int(f)) ? int(f) + 1 : int(f) }')
  fi
  [ "$refine" -lt 1 ] && refine=1
  targetRes=$(awk -v a="$hgtRes" -v f="$refine" 'BEGIN { printf "%.12f", a / f }')
  echo "overTif ${overRes} deg/px, hgt ${hgtRes} deg/px -> refine x${refine}, output ${targetRes} deg/px"

  #hgt layer, upsampled onto the target grid.
  #-srcnodata/-dstnodata replaces the old gdal_calc pass: hgt voids are excluded
  #from the interpolation instead of being clamped afterwards.
  #the warp keeps the hgt nodata as is, the -10 the rest of the chain expects is
  #set below: the hgt do contain real -10 pixels (rhone delta) and warping
  #straight to -10 makes gdal shift every one of them, one warning each.
  #-multi/-wo NUM_THREADS are serialized into the warped vrt, so the warp still
  #runs multi threaded when gdal_translate triggers it at the end
  gdalwarp $threadArgs -of VRT -overwrite -multi -wo NUM_THREADS=ALL_CPUS \
    -te $ulx $lry $lrx $uly -tr $targetRes $targetRes -ot $outType -r bilinear \
    -srcnodata -32768 -dstnodata -32768 elevation_tiles.vrt ${warpVrt}

  #gdalbuildvrt silently skips a source whose color interpretation differs, and
  #gdalwarp leaves its band Undefined
  gdal_translate -q -of VRT -colorinterp gray -a_nodata -10 ${warpVrt} ${warpGrayVrt}

  #the tilezen hgt carry bathymetry (down to -302m off the rhone delta) and the
  #hgt voids are still -32768 here. a vrt LUT clamps both to -10, which is what
  #the old `((A+10)*(A+10>0))-10` gdal_calc pass did, but without writing a raster
  sed -e 's|<SimpleSource>|<ComplexSource>|' \
      -e 's|</SimpleSource>|<LUT>-32768:-10,-10:-10,9000:9000</LUT></ComplexSource>|' \
      ${warpGrayVrt} > ${warpGrayVrt}.tmp && mv ${warpGrayVrt}.tmp ${warpGrayVrt}

  #overTif reprojected onto that very same grid
  gdalwarp $threadArgs -of VRT -overwrite -multi -wo NUM_THREADS=ALL_CPUS \
    -t_srs EPSG:4326 -te $ulx $lry $lrx $uly -tr $targetRes $targetRes -ot $outType -r bilinear \
    -srcnodata "$overNodata" -dstnodata "$overNodata" ${overTif} ${tempOverVrt}
  gdal_translate -q -of VRT -colorinterp gray ${tempOverVrt} ${tempOverGrayVrt}

  if [ "$blurMeters" = "0" ]; then
    #we finally merge both with overTif on top, single materialization.
    #-te pins the mosaic to the hgt bounds, each source keeps its own nodata
    #transparent so overTif wins where it has data and the hgt shows through elsewhere
    gdalbuildvrt -te $ulx $lry $lrx $uly -vrtnodata -10 merged.vrt ${warpGrayVrt} ${tempOverGrayVrt}
    gdal_translate $threadArgs -ot $outType $coArgs merged.vrt $output
    rm merged.vrt
  else
    #a hard overTif/hgt boundary shows up as a ridge in the hillshade: the two
    #sources disagree there (different resolution, and EGM96 vs NGF-IGN69 vertical
    #datum). so we cross fade them over ${blurMeters}m instead of stacking them.
    #the fade weight is the overTif coverage, averaged down to a coarse grid
    #(-dstalpha gives the fractional coverage, and the warp reads the overTif
    #overviews at that resolution so it stays cheap) then splined back up: the
    #downsample/upsample pair is the low pass filter, no convolution needed
    coarseRes=$(awk -v m="$blurMeters" -v t="$targetRes" 'BEGIN { c = m / 111320.0; printf "%.12f", (c < t) ? t : c }')
    echo "blur ${blurMeters}m -> coverage mask at ${coarseRes} deg/px"
    gdalwarp $threadArgs -q -of VRT -overwrite -t_srs EPSG:4326 \
      -te $ulx $lry $lrx $uly -tr $coarseRes $coarseRes -r average \
      -srcnodata "$overNodata" -dstalpha ${overTif} ${maskCoarseVrt}
    #-colorinterp gray is required: the extracted band keeps ColorInterp=Alpha
    #otherwise, and the upsampling warp below then treats it as an alpha channel
    #and hard masks with it instead of interpolating it
    gdal_translate -q -of VRT -b 2 -colorinterp gray ${maskCoarseVrt} ${maskAlphaVrt}
    gdalwarp $threadArgs -q -of VRT -overwrite \
      -te $ulx $lry $lrx $uly -tr $targetRes $targetRes -r cubicspline \
      ${maskAlphaVrt} ${maskFineVrt}

    #A hgt, B overTif, C coverage 0-255. where overTif is nodata its weight is 0,
    #so B never contributes there and A shows through untouched
    GDAL_NUM_THREADS=ALL_CPUS GDAL_CACHEMAX=$gdalCacheMax \
      gdal_calc.py --overwrite $coArgsPy --type=$outType --NoDataValue=-10 --hideNoData \
        -A ${warpGrayVrt} -B ${tempOverGrayVrt} -C ${maskFineVrt} \
        --calc "A*(1.0-(C/255.0)*(B!=(${overNodata})))+B*((C/255.0)*(B!=(${overNodata})))" \
        --outfile=$output
    rm ${maskCoarseVrt} ${maskAlphaVrt} ${maskFineVrt}
  fi
  rm ${warpVrt} ${warpGrayVrt}
  rm ${tempOverVrt} ${tempOverGrayVrt}
else
  #we compute NoDataValue straight from the vrt, no intermediate tif
  GDAL_NUM_THREADS=ALL_CPUS GDAL_CACHEMAX=$gdalCacheMax \
    gdal_calc.py --overwrite $coArgsPy --type=$outType -A elevation_tiles.vrt --outfile=$output --calc="((A+10)*(A+10>0))-10" --NoDataValue=-10
fi

rm elevation_tiles.vrt
