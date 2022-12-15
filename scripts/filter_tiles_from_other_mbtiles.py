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

def optimizeTiles(outputFileName,inputFileName):
  wantedtiles = set()
  tiles = []
  hasImageTable = False
  hasShallowTable = False
  with closing(sqlite3.connect(inputFileName)) as outputDb:
    # Harvest tiles
    cursor1 = outputDb.cursor()

    # Find tiles at specified zoom levels equal to their parent tiles
    cursor1.execute("SELECT tile_column, tile_row, zoom_level FROM tiles")
    for row in cursor1.fetchall():
      # Now check that there are no child tiles. In that case the tile can be deleted
      x, y, zoom = row
      wantedtiles.add((x, y, zoom))
    cursor1.close()
    outputDb.commit()
  print("wantedtiles %s", len(wantedtiles))
  # Drop tiles that are not needed
  with closing(sqlite3.connect(outputFileName)) as outputDb:
    # Harvest tiles
    cursor1 = outputDb.cursor()
    cursor1.execute("SELECT name FROM sqlite_master WHERE name ='images'")
    result = cursor1.fetchone()
    hasImageTable = result is not None and len(result) > 0
    if (not hasImageTable): 
      cursor1.execute("SELECT name FROM sqlite_master WHERE name ='tiles_shallow'")
      result = cursor1.fetchone()
      hasShallowTable = result is not None and len(result) > 0
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

  # if(len(filtered) > 0):
  with closing(sqlite3.connect(outputFileName)) as outputDb:
    cursor2 = outputDb.cursor()
    for x, y, zoom in filtered:
      if hasImageTable:
        cursor2.execute("SELECT tile_id FROM map WHERE zoom_level=? AND tile_column=? AND tile_row=?", (zoom, x, y))
        tile_id = cursor2.fetchone()[0]
        cursor2.execute("DELETE FROM images WHERE tile_id=?", (tile_id,))
        cursor2.execute("DELETE FROM map WHERE zoom_level=? AND tile_column=? AND tile_row=?", (zoom, x, y))
      elif hasShallowTable:
        cursor2.execute("SELECT tile_data_id FROM tiles_shallow WHERE zoom_level=? AND tile_column=? AND tile_row=?", (zoom, x, y))
        tile_id = cursor2.fetchone()[0]
        cursor2.execute("DELETE FROM tiles_data WHERE tile_data_id=?", (tile_id,))
        cursor2.execute("DELETE FROM tiles_shallow WHERE zoom_level=? AND tile_column=? AND tile_row=?", (zoom, x, y))
      else:
        cursor2.execute("DELETE FROM tiles WHERE zoom_level=? AND tile_column=? AND tile_row=?", (zoom, x, y))
    
    if hasImageTable:
      cursor2.execute("DELETE FROM images WHERE tile_id NOT IN (SELECT tile_id FROM map)")
    elif hasShallowTable:
      cursor2.execute("DELETE FROM tiles_data WHERE tile_data_id NOT IN (SELECT tile_data_id FROM tiles_shallow)")
    cursor2.close()
    outputDb.commit()
  print("removed filtered %s", len(filtered))

  #Vacuum
  with closing(sqlite3.connect(outputFileName)) as outputDb:
    outputDb.execute("VACUUM")

def main():
  parser = argparse.ArgumentParser()
  parser.add_argument('--sourcembtiles', dest='sourcembtiles', help='mbtiles to filter "from"')
  parser.add_argument(dest='mbtiles', help='mbtiles to update')
  # parser.add_argument(dest='output', help='Output directory for packages.json.template files')
  args = parser.parse_args()

  optimizeTiles(args.mbtiles, args.sourcembtiles)

if __name__ == "__main__":
  main()
