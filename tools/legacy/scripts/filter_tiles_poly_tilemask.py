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
import subprocess

def optimizeTiles(outputFileName,tilemask, maxzoom):
  wantedtiles = set(tilemask.tileMaskTiles(tilemask, maxzoom))
  print("wantedtiles %s", len(wantedtiles))
  # Drop tiles that are not needed
  with closing(sqlite3.connect(outputFileName)) as outputDb:
    # Harvest tiles
    cursor1 = outputDb.cursor()
    tiles = []
    # Find tiles at specified zoom levels equal to their parent tiles
    cursor1.execute("SELECT tile_column, tile_row, zoom_level FROM tiles")
    for row in cursor1.fetchall():
      # Now check that there are no child tiles. In that case the tile can be deleted
      x, y, zoom = row
      tiles += [(x, y, zoom)]
    cursor1.close()
    outputDb.commit()
  print("tiles %s", len(tiles))

  filtered = list(filter(lambda x: x not in wantedtiles, tiles))
  print("filtered %s", len(filtered))

  if(len(filtered) > 0):
    with closing(sqlite3.connect(outputFileName)) as outputDb:
      cursor2 = outputDb.cursor()
      for x, y, zoom in filtered:
        cursor2.execute("DELETE FROM tiles WHERE zoom_level=? AND tile_column=? AND tile_row=?", (zoom, x, y))
      
      cursor2.close()
      outputDb.commit()
    print("removed filtered %s", len(filtered))

    #Vacuum
    with closing(sqlite3.connect(outputFileName)) as outputDb:
      outputDb.execute("VACUUM")

def main():
  parser = argparse.ArgumentParser()
  parser.add_argument('--tilemask', dest='tilemask', help='Input tilemask string')
  parser.add_argument('--poly', dest='poly', help='Input .poly file')
  parser.add_argument('--polymaxzoom', dest='polymaxzoom', help='tilemask maxzoom', type=int)
  parser.add_argument('--maxzoom', dest='maxzoom', help='filtermaxzoom', type=int)
  parser.add_argument(dest='mbtiles', help='mbtiles to update')
  # parser.add_argument(dest='output', help='Output directory for packages.json.template files')
  args = parser.parse_args()

  tilemask =  args.tilemask
  polymaxzoom =  args.polymaxzoom
  maxzoom = args.maxzoom
  if not maxzoom:
    with closing(sqlite3.connect(args.mbtiles)) as packageDb:
      packageCursor = packageDb.cursor()
      packageCursor.execute("SELECT value FROM metadata WHERE name='maxzoom'")
      maxzoom = int(packageCursor.fetchone()[0])
      packageCursor.close()
      packageDb.commit()
  
  if args.poly:
    tilemask = subprocess.run(["python",("%s/generate_poly_tilemask.py" % (os.path.dirname(__file__))),("--poly=%s" % ( args.poly)),("--maxzoom=%s" % (polymaxzoom))], stdout=subprocess.PIPE).stdout.decode('utf-8')

  if not tilemask:
    with closing(sqlite3.connect(args.mbtiles)) as packageDb:
      packageCursor = packageDb.cursor()
      packageCursor.execute("SELECT value FROM metadata WHERE name='tilemask'")
      tilemask = packageCursor.fetchone()[0]
      packageCursor.close()
      packageDb.commit()

  print("tilemask %s", tilemask)
  print("maxzoom %s", maxzoom)
  if tilemask:
    optimizeTiles(args.mbtiles, tilemask, maxzoom)

if __name__ == "__main__":
  main()
