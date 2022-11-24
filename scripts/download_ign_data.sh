#!/bin/bash

SCREEN_OPTS=""
DOWNLOAD_DIR="./ign_data"

 downloadData()
{
  mkdir -p $DOWNLOAD_DIR
  pushd $DOWNLOAD_DIR

  if [[ ! -t 1 ]]; then
      SCREEN_OPTS="-d -m"
  fi

  screen $SCREEN_OPTS /usr/local/bin/wget --directory-prefix="." ftp://BD_ALTI_ext:docoazeecoosh1Ai@ftp3.ign.fr:/BDALTIV2_2-0_25M*MNT_LAMB93*


  # for file in ./*
  # do
  #     if [[ -f $file ]]; then
  #       7z e $file "*.asc" -r -aoa
  #       rm -f $file
  #     fi
  # done
  popd
}

 buildTif()
{
  find $DOWNLOAD_DIR -name '*MNT_LAMB93*.asc' > input_files.txt
  gdalbuildvrt -a_srs EPSG:2154 -hidenodata -input_file_list input_files.txt terrain.virt

  gdalwarp terrain.virt terrain.tif
  gdal_calc.py --co="COMPRESS=LZW" --type=Float32 -A  terrain.tif --outfile=terrain_0.tif --calc="((A+10)*(A+10>0))-10" --NoDataValue=-10
  echo "terrain.tif"
}


 buildRGBMBtiles() {
  rio rgbify -j 8 -b -10000 -i 0.1 --max-z 12 --min-z 5 --format webp $1 `(basename -s tif $1)`_webp.mbtiles
}

downloadData
# TERRAIN_TIF=buildTif
# buildRGBMBtiles $TERRAIN_TIF