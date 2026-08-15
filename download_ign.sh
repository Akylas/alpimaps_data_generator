#!/usr/bin/env bash
set -euo pipefail

############################################################
# macOS end-to-end IGN RGE ALTI 5m -> France GeoTIFF
# URLs are embedded directly in this script.
#
# Output:
#   ./work_france_ign/out/france_metropole_corse_rgealti5m_EPSG_2154.tif
#
# Usage:
#   chmod +x build_france_ign_embedded_urls_macos.sh
#   ./build_france_ign_embedded_urls_macos.sh
#
# Optional env vars:
#   WORKDIR=./work_france_ign
#   THREADS=ALL_CPUS
#   OUT_CRS=EPSG:2154
#   KEEP_INTERMEDIATE=1
############################################################

WORKDIR="${WORKDIR:-./work_france_ign}"
DOWNLOAD_DIR="$WORKDIR/download"
EXTRACT_DIR="$WORKDIR/extracted"
TIF_DIR="$WORKDIR/tif_tiles"
VRT_DIR="$WORKDIR/vrt"
OUT_DIR="$WORKDIR/out"
TMP_DIR="$WORKDIR/tmp"

THREADS="${THREADS:-ALL_CPUS}"
OUT_CRS="${OUT_CRS:-EPSG:2154}"
KEEP_INTERMEDIATE="${KEEP_INTERMEDIATE:-1}"

mkdir -p "$DOWNLOAD_DIR" "$EXTRACT_DIR" "$TIF_DIR" "$VRT_DIR" "$OUT_DIR" "$TMP_DIR"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || return 1
}

brew_install_if_missing() {
  local cmd="$1"
  local formula="$2"
  if ! need_cmd "$cmd"; then
    echo "==> Installing $formula via Homebrew..."
    brew install "$formula"
  fi
}

echo "==> Checking Homebrew..."
if ! need_cmd brew; then
  echo "Homebrew is required. Install from: https://brew.sh"
  exit 1
fi

echo "==> Ensuring required tools are installed..."
brew_install_if_missing curl curl
brew_install_if_missing gdal gdal
brew_install_if_missing python3 python
brew_install_if_missing 7z p7zip

echo "==> Writing embedded URL list..."
cat > "$TMP_DIR/urls_clean.txt" <<'EOF'
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D001_2023-08-08/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D001_2023-08-08.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D002_2020-09-04/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D002_2020-09-04.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D003_2023-08-10/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D003_2023-08-10.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D004_2023-08-08/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D004_2023-08-08.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D005_2020-10-14/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D005_2020-10-14.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D006_2023-08-08/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D006_2023-08-08.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D007_2022-12-16/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D007_2022-12-16.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D008_2019-10-14/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D008_2019-10-14.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D009_2023-10-04/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D009_2023-10-04.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D010_2021-11-04/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D010_2021-11-04.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D011_2023-10-04/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D011_2023-10-04.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D012_2022-09-29/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D012_2022-09-29.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D013_2022-12-16/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D013_2022-12-16.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D014_2022-12-21/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D014_2022-12-21.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D015_2022-09-29/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D015_2022-09-29.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D016_2023-07-28/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D016_2023-07-28.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D017_2023-07-28/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D017_2023-07-28.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D018_2023-01-03/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D018_2023-01-03.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D019_2019-09-09/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D019_2019-09-09.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D021_2023-08-08/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D021_2023-08-08.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D022_2022-10-14/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D022_2022-10-14.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D023_2019-11-20/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D023_2019-11-20.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D024_2019-10-17/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D024_2019-10-17.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D025_2021-01-13/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D025_2021-01-13.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D026_2022-12-16/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D026_2022-12-16.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D027_2022-12-21/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D027_2022-12-21.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D028_2019-12-16/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D028_2019-12-16.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D029_2022-10-14/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D029_2022-10-14.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D030_2022-12-16/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D030_2022-12-16.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D031_2021-05-12/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D031_2021-05-12.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D032_2019-11-21/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D032_2019-11-21.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D033_2021-04-19/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D033_2021-04-19.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D034_2022-12-16/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D034_2022-12-16.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D035_2022-11-15/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D035_2022-11-15.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D036_2022-09-28/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D036_2022-09-28.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D037_2023-07-20/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D037_2023-07-20.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D038_2020-11-13/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D038_2020-11-13.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D039_2023-08-08/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D039_2023-08-08.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D040_2021-04-19/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D040_2021-04-19.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D041_2019-12-16/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D041_2019-12-16.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D042_2023-08-10/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D042_2023-08-10.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D043_2022-10-03/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D043_2022-10-03.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D044_2022-12-20/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D044_2022-12-20.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D045_2023-01-03/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D045_2023-01-03.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D046_2019-10-17/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D046_2019-10-17.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D047_2019-11-21/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D047_2019-11-21.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D048_2022-12-16/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D048_2022-12-16.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D049_2023-01-12/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D049_2023-01-12.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D050_2022-12-21/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D050_2022-12-21.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D051_2019-10-14/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D051_2019-10-14.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D052_2021-01-13/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D052_2021-01-13.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D053_2023-01-12/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D053_2023-01-12.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D054_2021-09-24/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D054_2021-09-24.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D055_2018-12-07/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D055_2018-12-07.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D056_2022-12-15/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D056_2022-12-15.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D057_2021-09-24/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D057_2021-09-24.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D058_2023-08-01/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D058_2023-08-01.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D059_2021-09-20/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D059_2021-09-20.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D060_2020-09-04/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D060_2020-09-04.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D061_2023-01-12/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D061_2023-01-12.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D062_2021-09-20/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D062_2021-09-20.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D063_2021-01-22/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D063_2021-01-22.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D064_2021-04-19/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D064_2021-04-19.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D065_2020-02-11/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D065_2020-02-11.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D066_2023-10-04/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D066_2023-10-04.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D067_2021-11-02/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D067_2021-11-02.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D068_2021-11-02/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D068_2021-11-02.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D069_2023-08-10/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D069_2023-08-10.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D070_2021-01-13/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D070_2021-01-13.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D071_2023-08-10/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D071_2023-08-10.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D072_2023-01-12/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D072_2023-01-12.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D073_2020-10-15/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D073_2020-10-15.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D074_2020-10-15/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D074_2020-10-15.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D075_2020-07-30/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D075_2020-07-30.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D076_2020-10-20/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D076_2020-10-20.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D077_2021-03-03/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D077_2021-03-03.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D078_2020-07-30/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D078_2020-07-30.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D079_2023-08-10/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D079_2023-08-10.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D080_2019-07-09/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D080_2019-07-09.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D081_2022-07-29/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D081_2022-07-29.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D082_2019-10-30/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D082_2019-10-30.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D083_2022-12-05/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D083_2022-12-05.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D084_2022-12-16/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D084_2022-12-16.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D085_2023-07-28/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D085_2023-07-28.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D086_2023-08-10/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D086_2023-08-10.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D087_2021-10-26/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D087_2021-10-26.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D088_2021-11-02/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D088_2021-11-02.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D089_2023-01-03/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D089_2023-01-03.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D090_2021-01-13/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D090_2021-01-13.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D091_2021-03-03/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D091_2021-03-03.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D092_2020-07-30/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D092_2020-07-30.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D093_2020-07-30/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D093_2020-07-30.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D094_2020-07-30/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D094_2020-07-30.7z
https://data.geopf.fr/telechargement/download/RGEALTI/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D095_2020-07-30/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D095_2020-07-30.7z
EOF

echo "==> Downloading archives..."
while IFS= read -r url; do
  [ -z "$url" ] && continue
  fname="$(basename "$url")"
  dst="$DOWNLOAD_DIR/$fname"
  if [ -f "$dst" ]; then
    echo "Already downloaded: $fname"
  else
    echo "Downloading: $fname"
    curl -fL --retry 6 --retry-delay 2 --retry-all-errors -o "$dst" "$url"
  fi
done < "$TMP_DIR/urls_clean.txt"

echo "==> Extracting .7z archives..."
find "$DOWNLOAD_DIR" -type f -name "*.7z" | sort > "$TMP_DIR/all_7z.txt"
while IFS= read -r archive; do
  [ -z "$archive" ] && continue
  base="$(basename "$archive" .7z)"
  target="$EXTRACT_DIR/$base"
  marker="$target/.ok"
  mkdir -p "$target"
  if [ -f "$marker" ]; then
    echo "Already extracted: $(basename "$archive")"
    continue
  fi
  echo "Extracting: $(basename "$archive")"
  7z x -y "-o$target" "$archive" >/dev/null
  touch "$marker"
done < "$TMP_DIR/all_7z.txt"

echo "==> Finding ASC files..."
find "$EXTRACT_DIR" -type f \( -iname "*.asc" -o -iname "*.ASC" \) | sort > "$TMP_DIR/all_asc.txt"

if [ ! -s "$TMP_DIR/all_asc.txt" ]; then
  echo "No ASC files found after extraction."
  exit 1
fi

echo "==> Converting ASC -> GeoTIFF (EPSG:2154)..."
python3 <<'PY'
from pathlib import Path
import subprocess

workdir = Path("work_france_ign")
asc_list = workdir / "tmp" / "all_asc.txt"
tif_dir = workdir / "tif_tiles"
tif_dir.mkdir(parents=True, exist_ok=True)

files = [l.strip() for l in asc_list.read_text().splitlines() if l.strip()]
total = len(files)
print(f"Total ASC files: {total}")

for i, asc in enumerate(files, 1):
    asc_p = Path(asc)
    tif_p = tif_dir / (asc_p.stem + ".tif")
    if tif_p.exists():
        continue

    cmd = [
        "gdal_translate",
        "-of", "GTiff",
        "-a_srs", "EPSG:2154",
        "-ot", "Float32",
        "-co", "TILED=YES",
        "-co", "COMPRESS=ZSTD",
        "-co", "PREDICTOR=3",
        "-co", "BIGTIFF=IF_SAFER",
        str(asc_p),
        str(tif_p),
    ]
    subprocess.run(cmd, check=True)

    if i % 200 == 0:
        print(f"{i}/{total} converted")

print("Conversion finished.")
PY

echo "==> Building VRT mosaic..."
find "$TIF_DIR" -type f -name "*.tif" | sort > "$TMP_DIR/all_tif.txt"
gdalbuildvrt -input_file_list "$TMP_DIR/all_tif.txt" "$VRT_DIR/france_ign_rgealti5m.vrt"

echo "==> Writing final GeoTIFF..."
FINAL_TIF="$OUT_DIR/france_metropole_corse_rgealti5m_${OUT_CRS//:/_}.tif"

gdalwarp \
  -t_srs "$OUT_CRS" \
  -r bilinear \
  -dstnodata -99999 \
  -multi -wo NUM_THREADS="$THREADS" \
  -co TILED=YES \
  -co COMPRESS=ZSTD \
  -co PREDICTOR=3 \
  -co BIGTIFF=YES \
  "$VRT_DIR/france_ign_rgealti5m.vrt" \
  "$FINAL_TIF"

echo "==> Building overviews..."
gdaladdo -r average "$FINAL_TIF" 2 4 8 16 32

if [ "$KEEP_INTERMEDIATE" = "0" ]; then
  echo "==> Cleaning intermediate folders..."
  rm -rf "$EXTRACT_DIR" "$TIF_DIR" "$VRT_DIR" "$TMP_DIR"
fi

echo ""
echo "✅ Done"
echo "Output:"
echo "  $FINAL_TIF"