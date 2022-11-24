#!/bin/bash
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

export PATH=$SCRIPT_DIR/venv/bin:$SCRIPT_DIR/tippecanoe:$SCRIPT_DIR/valhalla/build:$SCRIPT_DIR/prime_server:$PATH
export OUTPUT_DIR=alpimaps_mbtiles
