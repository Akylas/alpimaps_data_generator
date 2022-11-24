# Script for extracting valhalla tiles (from Valhalla tile directory structure) into .vtiles sqlite
# databases.

import io
import os
import sys
import json
import gzip
import zlib
import base64
import math
import sqlite3
import argparse
import subprocess
import concurrent.futures
import utils.pyproj_lite as pyproj
from contextlib import closing

# Zoom level/precision for tilemasks
TILEMASK_ZOOM = 10

# Generic projection values
VALHALLA_BOUNDS = ((-180, -90), (180, 90))
VALHALLA_TILESIZES = [4.0, 1.0, 0.25]
MERCATOR_BOUNDS = ((-6378137 * math.pi, -6378137 * math.pi), (6378137 * math.pi, 6378137 * math.pi))

class PackageTileMask(object):
  def __init__(self, tileMaskStr):
    self.data = self._decodeTileMask(tileMaskStr)
    self.rootNode = self._buildTileNode(list(self.data), (0, 0, 0))

  def contains(self, tile):
    node = self._findTileNode(tile)
    if node is None:
      return False
    return node["inside"]

  def getTiles(self, maxZoom=None):
    tiles = []
    if self.data != []:
      self._buildTiles(list(self.data), (0, 0, 0), maxZoom, tiles)
    return tiles

  def _decodeTileMask(self, tileMaskStr):
    str = [c for c in base64.b64decode(tileMaskStr)]
    data = []
    for i in range(len(str) * 8):
      val = (str[i // 8] >> (7 - i % 8)) & 1
      data.append(val)
    return data

  def _buildTileNode(self, data, tile):
    (zoom, x, y) = tile
    subtiles = data.pop(0)
    inside = data.pop(0)
    node = { "tile" : tile, "inside": inside, "subtiles": [] }
    if subtiles:
      for dy in range(0, 2):
        for dx in range(0, 2):
          node["subtiles"].append(self._buildTileNode(data, (zoom + 1, x * 2 + dx, y * 2 + dy)))
    return node

  def _findTileNode(self, tile):
    (zoom, x, y) = tile
    if zoom == 0:
      return self.rootNode if tile == (0, 0, 0) else None

    parentNode = self._findTileNode((zoom - 1, x >> 1, y >> 1))
    if parentNode:
      for node in parentNode["subtiles"]:
        if node["tile"] == tile:
          return node
      if parentNode["inside"]:
        return parentNode
    return None

  def _buildTiles(self, data, tile, maxZoom, tiles):
    (zoom, x, y) = tile
    submask = data.pop(0)
    inside = data.pop(0)
    if inside:
      tiles.append(tile)
    if submask:
      for dy in range(0, 2):
        for dx in range(0, 2):
          self._buildTiles(data, (zoom + 1, x * 2 + dx, y * 2 + dy), maxZoom, tiles)
    elif maxZoom is not None and inside:
      for dy in range(0, 2):
        for dx in range(0, 2):
          self._buildAllTiles((zoom + 1, x * 2 + dx, y * 2 + dy), maxZoom, tiles)

  def _buildAllTiles(self, tile, maxZoom, tiles):
    (zoom, x, y) = tile
    if zoom > maxZoom:
      return
    tiles.append(tile)
    for dy in range(0, 2):
      for dx in range(0, 2):
        self._buildAllTiles((zoom + 1, x * 2 + dx, y * 2 + dy), maxZoom, tiles)

def valhallaTilePath(vTile):
  vTileSize = VALHALLA_TILESIZES[vTile[2]]
  r = int((VALHALLA_BOUNDS[1][0] - VALHALLA_BOUNDS[0][0]) / vTileSize)
  id = vTile[1] * r + vTile[0]
  splitId = []
  for i in range(0, max(1, vTile[2]) + 1):
    splitId = ['%03d' % (id % 1000)] + splitId
    id /= 1000
  splitId = [str(vTile[2])] + splitId
  return '/'.join(splitId) + '.gph'

def _calculateValhallaTiles(mTile, vZoom, transformer):
  mTileSize = (MERCATOR_BOUNDS[1][0] - MERCATOR_BOUNDS[0][0]) / (1 << mTile[2])
  vTileSize = VALHALLA_TILESIZES[vZoom]
  mX0, mY0 = mTile[0] * mTileSize + MERCATOR_BOUNDS[0][0], mTile[1] * mTileSize + MERCATOR_BOUNDS[0][1]
  mX1, mY1 = mX0 + mTileSize, mY0 + mTileSize
  vY0, vX0 = transformer.transform(mX0, mY0)
  vY1, vX1 = transformer.transform(mX1, mY1)
  vTile0 = (vX0 - VALHALLA_BOUNDS[0][0]) / vTileSize, (vY0 - VALHALLA_BOUNDS[0][1]) / vTileSize
  vTile1 = (vX1 - VALHALLA_BOUNDS[0][0]) / vTileSize, (vY1 - VALHALLA_BOUNDS[0][1]) / vTileSize
  vTiles = []
  for y in range(int(math.floor(vTile0[1])), int(math.ceil(vTile1[1]))):
    for x in range(int(math.floor(vTile0[0])), int(math.ceil(vTile1[0]))):
      vTiles.append((x, y, vZoom))
  return vTiles

def calculateValhallaTilesFromTileMask(tileMask, polyzoom):
  vTiles = set()
  mTiles = [(x, y, zoom) for zoom, x, y in PackageTileMask(tileMask).getTiles(polyzoom)]
  transformer = pyproj.Transformer.from_crs('EPSG:3857', 'EPSG:4326')
  for mTile in mTiles:
    if mTile[2] < TILEMASK_ZOOM:
      continue
    for vZoom, vTileSize in enumerate(VALHALLA_TILESIZES):
      for vTile in _calculateValhallaTiles(mTile, vZoom, transformer):
        vTiles.add(vTile)
  return sorted(list(vTiles))

def compressTile(tileData):
  compress = zlib.compressobj(9, zlib.DEFLATED, 31, 9, zlib.Z_DEFAULT_STRATEGY)
  deflated = compress.compress(tileData)
  deflated += compress.flush()
  return deflated

def extractTiles(packageId, tileMask, outputFileName, valhallaTileDir, polyzoom):
  if os.path.exists(outputFileName):
    os.remove(outputFileName)

  with closing(sqlite3.connect(outputFileName)) as outputDb:
    outputDb.execute("PRAGMA locking_mode=EXCLUSIVE")
    outputDb.execute("PRAGMA synchronous=OFF")
    outputDb.execute("PRAGMA page_size=512")
    outputDb.execute("PRAGMA encoding='UTF-8'")

    cursor = outputDb.cursor();
    cursor.execute("CREATE TABLE metadata (name TEXT, value TEXT)");
    cursor.execute("CREATE TABLE tiles (zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_data BLOB)");
    cursor.execute("INSERT INTO metadata(name, value) VALUES('name', ?)", (packageId,))
    cursor.execute("INSERT INTO metadata(name, value) VALUES('type', 'routing')")
    cursor.execute("INSERT INTO metadata(name, value) VALUES('version', '1.0')")
    cursor.execute("INSERT INTO metadata(name, value) VALUES('description', 'Nutiteq Valhalla routing package for ' || ?)", (packageId,))
    cursor.execute("INSERT INTO metadata(name, value) VALUES('format', 'gph3')")

    vTiles = calculateValhallaTilesFromTileMask(tileMask, polyzoom)
    for vTile in vTiles:
      file = os.path.join(valhallaTileDir, valhallaTilePath(vTile))
      if os.path.isfile(file):
        print('handling File %s' % file)
        with closing(io.open(file, 'rb')) as sourceFile:
          compressedData = compressTile(sourceFile.read())
          cursor.execute("INSERT INTO tiles(zoom_level, tile_column, tile_row, tile_data) VALUES(?, ?, ?, ?)", (vTile[2], vTile[0], vTile[1], bytes(compressedData)));
      else:
        print('Warning: File %s does not exist!' % file)

    cursor.execute("CREATE UNIQUE INDEX tiles_index ON tiles (zoom_level, tile_column, tile_row)");
    cursor.close()
    outputDb.commit()

  with closing(sqlite3.connect(outputFileName)) as outputDb:
    outputDb.execute("VACUUM")

def processPackage(package_id, tilemask , outputFileName, tilesDir, polyzoom):
  if os.path.exists(outputFileName):
    if not os.path.exists(outputFileName + "-journal"):
      return outputFileName
    os.remove(outputFileName)
    os.remove(outputFileName + "-journal")

  print('Processing %s' % package_id)
  try:
    extractTiles(package_id, tilemask, outputFileName, tilesDir, polyzoom)
  except:
    if os.path.isfile(outputFileName):
      os.remove(outputFileName)
    raise
  return outputFileName
    
def main():
  parser = argparse.ArgumentParser()
  parser.add_argument(dest='input', help='directory for Valhalla tiles')
  parser.add_argument(dest='output', help='output directory for packages')
  parser.add_argument('--id', dest='package_id', default=None, help='package id (area name)')
  parser.add_argument('--tilemask', dest='tilemask', help='Input tilemask string')
  parser.add_argument('--poly', dest='poly', help='Input .poly file')
  parser.add_argument('--polymaxzoom', dest='polymaxzoom', help='tilemask maxzoom', type=int)
  args = parser.parse_args()

  if (not args.tilemask and args.poly):
    args.tilemask = subprocess.run(["python",("%s/generate_poly_tilemask.py" % (os.path.dirname(__file__))),("--poly=%s" % ( args.poly)),("--maxzoom=%s" % (args.polymaxzoom))], stdout=subprocess.PIPE).stdout.decode('utf-8')


  processPackage(args.package_id, args.tilemask, args.output, args.input, args.polymaxzoom)

if __name__ == "__main__":
  main()
