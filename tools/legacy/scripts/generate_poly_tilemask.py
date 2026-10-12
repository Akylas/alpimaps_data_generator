# Script for building tilemasks and packages.json template from .poly files

import io
import os
import glob
import json
import argparse
import utils.polygon2geojson as polygon2geojson
import utils.tilemask as tilemask
import functools
import re
import geojson
import sys
import sqlite3
from contextlib import closing

TILEMASK_SIZE_THRESHOLD = 512

def main():
  parser = argparse.ArgumentParser()
  parser.add_argument('--poly', dest='poly', help='Input .poly file')
  parser.add_argument('--maxzoom', dest='maxzoom', default=14, help='tilemask maxzoom')
  parser.add_argument(dest='mbtiles', nargs='?', help='mbtiles to update')
  # parser.add_argument(dest='output', help='Output directory for packages.json.template files')
  args = parser.parse_args()

  polyFilename =  args.poly
  argMaxZoom = int(args.maxzoom)
  packageName = polyFilename.split("/")[-1][:-5].replace("_", "/")

  geojson_filename = polyFilename + ".geojson"
  polygon2geojson.main(polyFilename, geojson_filename)
  for maxZoom in range(argMaxZoom - 4, argMaxZoom + 1):
    mask = tilemask.processPolygon(polyFilename + ".geojson", maxZoom)
    if len(mask) >= TILEMASK_SIZE_THRESHOLD:
      break

  if (args.mbtiles):
    with closing(sqlite3.connect(args.mbtiles)) as packageDb:
      packageCursor = packageDb.cursor()
      packageCursor.execute("INSERT INTO metadata(name, value) VALUES('tilemask', ?)",(str(mask, 'utf8'),))
      packageCursor.close()
      packageDb.commit()
  # calculate stuff
  sys.stdout.write(str(mask, 'utf8'))
  sys.stdout.flush()
  return  str(mask, 'utf8');

if __name__ == "__main__":
  main()
