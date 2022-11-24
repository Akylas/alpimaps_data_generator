#!/usr/bin/bash
set -e
# set -o xtrace

source1=""
source2=""
output="output.tif"

ps="25"

while [ $# -gt 0 ]; do
  case "$1" in
    -o|--output=*)
      output="$2"
      shift
      ;;
    *)
    if [[ -n $source1 ]]; then
        source2="$1"
    else
        source1="$1"
    fi
  esac
  shift
done
[ "$source1" ] || usage

source1WithoutExt=${source1%.*}
source1Wwarped=${source1WithoutExt}_ps_${ps}.tif


echo "source1 $source1"
echo "source1Wwarped $source1Wwarped"
echo "source2 $source2"
echo "ps $ps"
echo "output $output"
gdalwarp -tr  ${ps} ${ps} ${source1} ${source1Wwarped}
gdal_merge.py -o ${output} -ps ${ps} ${ps} ${source1} ${source2}