#!/usr/bin/bash

set -e


# Download from http://files.opendatarchives.fr/professionnels.ign.fr/bdalti/
curl http://files.opendatarchives.fr/professionnels.ign.fr/bdalti/ | \
    grep _25M_ASC_ | cut -d '"' -f 2 | sed -e "s_^_http://files.opendatarchives.fr/professionnels.ign.fr/bdalti/_" \
    > url
# +5.3 GB, 20 min
wget -i url


# Extract and drop the archive
# 10 min, +8.0 GB, -5.3 GB
rm -fr asc && mkdir -p asc && \
ls *.7z | xargs -n 1 7zr e -oasc -aos && \
rm *.7z


# Convert ASC to GeoTiff
# 6 min, +2.8 GB
gdalbuildvrt -a_srs "EPSG:2154" -hidenodata BDALTIV2_2-0_25M_ASC_LAMB93.virt asc/*LAMB93*.asc
gdal_translate -co compress=lzw -of GTiff BDALTIV2_2-0_25M_ASC_LAMB93.virt BDALTIV2_2-0_25M_ASC_LAMB93.tif
rm BDALTIV2_2-0_25M_ASC_LAMB93.virt

# -8.0 GB
# rm -fr asc

# 2 min, +2.8 GB
# Set sea level to 0
gdal_calc.py --co="COMPRESS=LZW" --type=Float32 \
    -A BDALTIV2_2-0_25M_ASC_LAMB93.tif \
    --overwrite --outfile=BDALTIV2_2-0_25M_ASC_LAMB93_0.tif \
    --calc="((A+10)*(A+10>0))-10" --NoDataValue=-10

