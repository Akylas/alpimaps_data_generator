# Script for calculating tile mask with specified precision (zoom level)
# for given mbtiles file or package id. The resulting tile mask can be used for very fast
# 'point in tile package' queries. Current implementation assumes that
# if a tile is missing from MBTiles, then none of the subtiles also exists.
# This is a limitation of the script (performance optimization) and not
# a limitation of the encoding.

import os
import sys
import argparse
import base64
import json
from shapely.geometry import shape, Polygon, mapping
from shapely.prepared import prep
from shapely.ops import cascaded_union, unary_union, transform
from functools import partial

DEFAULT_MIN_ZOOM = 0
DEFAULT_MID_ZOOM = 0
DEFAULT_MAX_ZOOM = 14

TILEMASK_DEFAULT_MAX_ZOOM = 10

PROJECTION_SRID = 3857
PROJECTION_BOUNDS = (-20037508.34, -20037508.34, 20037508.34, 20037508.34)

try:
  from settings import *
except ImportError:
  pass

class TileExtractorFromPolygon(object):
  def __init__(self, polygonFile, srid, sridBounds, maxZoom):
    import pyproj

    self.srid = srid
    self.sridBounds = sridBounds
    self.maxZoom = maxZoom
    self.tileCounter = 0
    with open(polygonFile, 'r') as f:
      js = json.load(f)

    polygons = []

    # reproject geojson from wgs84 to Spherical Mercator
    transformer = pyproj.Transformer.from_crs('EPSG:4326', 'EPSG:3857')

    for feature in js['features']:
      polygons.append(shape(feature['geometry']).buffer(0))

    mergedPolygon = unary_union(polygons)

    # convert to spherical mercator (proj of tiles)
    transformedPolygon = transform(lambda x, y: transformer.transform(y, x), mergedPolygon)

    # preparing polygon makes intersect-queries 100x faster
    self.polygon = prep(transformedPolygon.simplify(20, preserve_topology=False))

  def extractTiles(self):
    self.tiles = []
    self.extractBoundTiles(self.sridBounds, [], True)
    return self.tiles

  def extractBoundTiles(self, tileBounds, tileId, testIntersection):
    # Do overlap/cover tests
    zoom = len(tileId)
    if testIntersection:
      # is given tile within given polygon?
      tileShape = Polygon([(tileBounds[0], tileBounds[1]), (tileBounds[0], tileBounds[3]), (tileBounds[2], tileBounds[3]), (tileBounds[2], tileBounds[1])])

      if not self.polygon.intersects(tileShape):
        return

      if zoom < self.maxZoom:
        if self.polygon.contains(tileShape):
          testIntersection = False

    # Calculate tile coordinates
    x, y = 0, 0
    for tx, ty in tileId:
      x = x * 2 + tx
      y = y * 2 + ty
    self.tiles.append((x, y, zoom))

    # If not at last zoom level, split tile into subtiles and process recursively
    if zoom < self.maxZoom:
      x0, y0, x2, y2 = tileBounds
      x1, y1 = (x0 + x2) / 2, (y0 + y2) / 2
      self.extractBoundTiles((x0, y0, x1, y1), tileId + [(0, 0)], testIntersection)
      self.extractBoundTiles((x1, y0, x2, y1), tileId + [(1, 0)], testIntersection)
      self.extractBoundTiles((x0, y1, x1, y2), tileId + [(0, 1)], testIntersection)
      self.extractBoundTiles((x1, y1, x2, y2), tileId + [(1, 1)], testIntersection)

class TileExtractor(object):
  def __init__(self, packageId, srid, sridBounds, maxZoom, dbConn):
    self.packageId = packageId
    self.srid = srid
    self.sridBounds = sridBounds
    self.maxZoom = maxZoom
    self.tileCounter = 0
    self.dbConn = dbConn

  def extractTiles(self):
    self.tiles = []
    self.extractBoundTiles(self.sridBounds, [], True)
    return self.tiles

  def extractBoundTiles(self, tileBounds, tileId, testIntersection):
    # Do overlap/cover tests
    zoom = len(tileId)
    if testIntersection:
      cursor = self.dbConn.cursor()
      baseSql = cursor.mogrify("SELECT package_id FROM packages_big WHERE package_id=%s", (self.packageId,))
      tileBoundsSql = "ST_SetSRID(ST_MakeBox2d(ST_MakePoint(%g, %g), ST_MakePoint(%g, %g)), %d)" % (tileBounds[0], tileBounds[1], tileBounds[2], tileBounds[3], self.srid)

      cursor.execute("%s AND ST_Intersects(geometry, %s)" % (baseSql, tileBoundsSql))
      if not cursor.fetchone():
        return
      if zoom < self.maxZoom:
        cursor.execute("%s AND ST_Contains(geometry, %s)" % (baseSql, tileBoundsSql))
        if cursor.fetchone():
          testIntersection = False

    # Calculate tile coordinates
    x, y = 0, 0
    for tx, ty in tileId:
      x = x * 2 + tx
      y = y * 2 + ty
    self.tiles.append((x, y, zoom))

    # If not at last zoom level, split tile into subtiles and process recursively
    if zoom < self.maxZoom:
      x0, y0, x2, y2 = tileBounds
      x1, y1 = (x0 + x2) / 2, (y0 + y2) / 2
      self.extractBoundTiles((x0, y0, x1, y1), tileId + [(0, 0)], testIntersection)
      self.extractBoundTiles((x1, y0, x2, y1), tileId + [(1, 0)], testIntersection)
      self.extractBoundTiles((x0, y1, x1, y2), tileId + [(0, 1)], testIntersection)
      self.extractBoundTiles((x1, y1, x2, y2), tileId + [(1, 1)], testIntersection)

def buildTileMask(tiles, x, y, zoom, maxZoom):
  if (x, y, zoom) not in tiles:
    return [0, 0]
  if zoom == maxZoom:
    return [0, 1]
  subTrees = []
  for dy in range(0, 2):
    for dx in range(0, 2):
      subTree = buildTileMask(tiles, x * 2 + dx, y * 2 + dy, zoom + 1, maxZoom)
      subTrees += subTree
  if subTrees == [0, 1, 0, 1, 0, 1, 0, 1]:
    return [0, 1] # Optimization, no need to store subtile data
  return [1, 1] + subTrees

def encodeTileMask(data):
  while len(data) % 24 != 0:
    data.append(0)
  str = bytearray()
  val = 0
  for i in range(len(data)):
    val = (val << 1) | data[i]
    if (i + 1) % 8 == 0:
      str.append(val)
      val = 0
  return base64.b64encode(str)

def decodeTileMask(tileMaskStr):
  tileMaskStr = tileMaskStr.replace('-', '+').replace('_', '/')
  missing_padding = len(tileMaskStr) % 4
  str = [c for c in base64.b64decode(tileMaskStr)]
  data = []
  for i in range(len(str) * 8):
    val = (str[i // 8] >> (7 - i % 8)) & 1
    data.append(val)
  return data

def _buildTiles(x, y, zoom, maxZoom):
  if zoom > maxZoom:
    return []
  tiles = [(x, y, zoom)]
  tiles += _buildTiles(x * 2 + 0, y * 2 + 0, zoom + 1, maxZoom)
  tiles += _buildTiles(x * 2 + 1, y * 2 + 0, zoom + 1, maxZoom)
  tiles += _buildTiles(x * 2 + 0, y * 2 + 1, zoom + 1, maxZoom)
  tiles += _buildTiles(x * 2 + 1, y * 2 + 1, zoom + 1, maxZoom)
  return tiles

def _tileMaskTiles(x, y, zoom, maxZoom, data):
  submask = data.pop(0)
  tiles = [(x, y, zoom)] if data.pop(0) else []
  if submask:
    tiles += _tileMaskTiles(x * 2 + 0, y * 2 + 0, zoom + 1, maxZoom, data)
    tiles += _tileMaskTiles(x * 2 + 1, y * 2 + 0, zoom + 1, maxZoom, data)
    tiles += _tileMaskTiles(x * 2 + 0, y * 2 + 1, zoom + 1, maxZoom, data)
    tiles += _tileMaskTiles(x * 2 + 1, y * 2 + 1, zoom + 1, maxZoom, data)
  elif maxZoom is not None:
    if maxZoom > zoom and (x, y, zoom) in tiles:
      tiles = _buildTiles(x, y, zoom, maxZoom)
  return tiles  

def tileMaskTiles(tileMaskStr, maxZoom=None):
  data = decodeTileMask(tileMaskStr)
  if data == []:
    return []
  return _tileMaskTiles(0, 0, 0, maxZoom, data)

def _tileMaskPolygon(x, y, zoom, tiles, parentTiles):
  if (x, y, zoom) in tiles:
    worldSizeX = PROJECTION_BOUNDS[2]-PROJECTION_BOUNDS[0]
    worldSizeY = PROJECTION_BOUNDS[3]-PROJECTION_BOUNDS[1]
    worldX0 = PROJECTION_BOUNDS[0] + worldSizeX / (1 << zoom) * x
    worldY0 = PROJECTION_BOUNDS[1] + worldSizeY / (1 << zoom) * y
    worldX1 = PROJECTION_BOUNDS[0] + worldSizeX / (1 << zoom) * (x + 1)
    worldY1 = PROJECTION_BOUNDS[1] + worldSizeY / (1 << zoom) * (y + 1)
    return Polygon([(worldX0, worldY0), (worldX1, worldY0), (worldX1, worldY1), (worldX0, worldY1)])
  if (x, y, zoom) in parentTiles:
    polys = [_tileMaskPolygon(x * 2 + dx, y * 2 + dy, zoom + 1, tiles, parentTiles) for dx, dy in [(0, 0), (1, 0), (1, 1), (0, 1)]]
    polys = list(filter(lambda poly:poly is not None, polys))
    return cascaded_union(polys) if polys else None
  return None

def tileMaskPolygon(tileMaskStr):
  tiles = tileMaskTiles(tileMaskStr)
  tiles = set(tiles)
  parentTiles = set()
  for x, y, zoom in list(tiles):
    while zoom > 0:
      x = x >> 1
      y = y >> 1
      zoom = zoom - 1
      tiles.discard((x, y, zoom))
      parentTiles.add((x, y, zoom))
  return _tileMaskPolygon(0, 0, 0, tiles, parentTiles)

def _tileMaskIntersection(x, y, zoom, data1, data2):
  submask1 = data1.pop(0)
  submask2 = data2.pop(0)
  inside1 = data1.pop(0)
  inside2 = data2.pop(0)
  tiles = [(x, y, zoom)] if inside1 and inside2 else []
  for dy in (0, 1):
    for dx in (0, 1):
      if submask1 and submask2:
        tiles += _tileMaskIntersection(x * 2 + dx, y * 2 + dy, zoom + 1, data1, data2)
      elif submask1:
        _tileMaskTiles(x * 2 + dx, y * 2 + dy, zoom + 1, None, data1)
      elif submask2:
        _tileMaskTiles(x * 2 + dx, y * 2 + dy, zoom + 1, None, data2)
  return tiles  

def tileMaskIntersection(tileMaskStr1, tileMaskStr2, maxZoom=TILEMASK_DEFAULT_MAX_ZOOM):
  data1 = decodeTileMask(tileMaskStr1)
  data2 = decodeTileMask(tileMaskStr2)
  if data1 == [] or data2 == []:
    tiles = []
  else:
    tiles = _tileMaskIntersection(0, 0, 0, data1, data2)
  return encodeTileMask(buildTileMask(tiles, 0, 0, 0, maxZoom))

def _tileMaskUnion(x, y, zoom, data1, data2):
  submask1 = data1.pop(0)
  submask2 = data2.pop(0)
  inside1 = data1.pop(0)
  inside2 = data2.pop(0)
  tiles = [(x, y, zoom)] if inside1 or inside2 else []
  for dy in (0, 1):
    for dx in (0, 1):
      if submask1 and submask2:
        tiles += _tileMaskUnion(x * 2 + dx, y * 2 + dy, zoom + 1, data1, data2)
      elif submask1:
        tiles += _tileMaskTiles(x * 2 + dx, y * 2 + dy, zoom + 1, None, data1)
      elif submask2:
        tiles += _tileMaskTiles(x * 2 + dx, y * 2 + dy, zoom + 1, None, data2)
  return tiles  

def tileMaskUnion(tileMaskStr1, tileMaskStr2, maxZoom=TILEMASK_DEFAULT_MAX_ZOOM):
  data1 = decodeTileMask(tileMaskStr1)
  data2 = decodeTileMask(tileMaskStr2)
  tiles = []
  if data1 == []:
    tiles = tileMaskTiles(data2)
  elif data2 == []:
    tiles = tileMaskTiles(data1)
  else:
    tiles = _tileMaskUnion(0, 0, 0, data1, data2)
  return encodeTileMask(buildTileMask(tiles, 0, 0, 0, maxZoom))

def processDb(packageId, maxZoom=TILEMASK_DEFAULT_MAX_ZOOM):
  import psycopg2

  # Create tile extractor
  dbConn = psycopg2.connect(DB_CONNECTION_PARAMS)
  extractor = TileExtractor(packageId, PROJECTION_SRID, PROJECTION_BOUNDS, maxZoom, dbConn)
  tiles = extractor.extractTiles()

  # Build tile tree, flatten it and binary-encode
  data = buildTileMask(set(tiles), 0, 0, 0, maxZoom)
  return encodeTileMask(data)

def processFileTilemask(mbTilesFile, maxZoom=TILEMASK_DEFAULT_MAX_ZOOM):
  import sqlite3

  # Find maximum tile zoom level
  mbTilesConn = sqlite3.connect(mbTilesFile)
  cursor = mbTilesConn.cursor()
  cursor.execute("SELECT MAX(zoom_level) FROM tiles")
  maxTileZoom = cursor.fetchone()[0]
  maxZoom = min(maxZoom, maxTileZoom)

  # Create tile list
  cursor.execute("SELECT tile_column, tile_row, zoom_level FROM tiles WHERE zoom_level <= ?", (maxZoom,))
  tiles = []
  for row in cursor.fetchall():
    x, y, zoom = row
    tiles.append((x, y, zoom))

  # Build tile tree, flatten it and binary-encode
  data = buildTileMask(set(tiles), 0, 0, 0, maxZoom)
  return encodeTileMask(data)

def processFileCsv(csvFileName, maxZoom=TILEMASK_DEFAULT_MAX_ZOOM):
  import csv
  
  # Read tile list from CSV
  tiles = []
  with open(csvFileName, 'rb') as csvfile:
    myreader = csv.reader(csvfile, delimiter='\t', quotechar='|')
    next(myreader) # skip header
    for row in myreader:
      x, y, zoom = int(row[0]), int(row[1]), int(row[2])
      tiles.append((x, y, zoom))
    
  # Build tile tree, flatten it and binary-encode
  data = buildTileMask(set(tiles), 0, 0, 0, maxZoom)
  return encodeTileMask(data)

def processPolygon(geojsonfile, maxZoom):
  # Create tile extractor
  extractor = TileExtractorFromPolygon(geojsonfile, PROJECTION_SRID, PROJECTION_BOUNDS, maxZoom)
  tiles = extractor.extractTiles()

  # Build tile tree, flatten it and binary-encode
  data = buildTileMask(set(tiles), 0, 0, 0, maxZoom)
  return encodeTileMask(data)

def _createWorld(filename, pixels):
  # Save World file as in https://en.wikipedia.org/wiki/World_file
  worldSizeX = PROJECTION_BOUNDS[2]-PROJECTION_BOUNDS[0]
  worldSizeY = PROJECTION_BOUNDS[3]-PROJECTION_BOUNDS[1]
  
  pixelSizeX =  worldSizeX / pixels
  pixelSizeY = -worldSizeY / pixels # has to be negative, as Y is from top here
  
  xOrigin = PROJECTION_BOUNDS[0] + (pixelSizeX / 2)
  yOrigin = PROJECTION_BOUNDS[3] + (pixelSizeY / 2)
  
  with open(filename, 'w') as target:
    target.write(str(pixelSizeX))
    target.write("\n")
    target.write("0") #xskew
    target.write("\n")
    target.write("0")
    target.write("\n") #yskew
    target.write(str(pixelSizeY))
    target.write("\n")
    target.write(str(xOrigin))
    target.write("\n")
    target.write(str(yOrigin))
  
def buildFileImage(tileMask, imageFile, preview=False, maxZoom=TILEMASK_DEFAULT_MAX_ZOOM):
  from PIL import Image

  # Create tile list
  tiles = tileMaskTiles(tileMask, maxZoom)

  # Create tile image
  imageSize = 2**maxZoom
  img = Image.new('RGB', (imageSize, imageSize), color='white') # create a new white image
  pixels = img.load() # create the pixel map
  for x, y, zoom in tiles:
    if zoom == maxZoom:
      pixels[x, imageSize - y - 1] = (0, 0, 0) # set the pixel color to black
    
  # Save image
  if imageFile is not None:
    if imageFile.endswith(".png"):
      img.save(imageFile, optimize=1)
    else:
      img.save(imageFile)
      
    # General World file ext pattern for 3-letter extensions: .png->pgw etc
    filename, ext = os.path.splitext(imageFile)
    worldExt = ext[1] + ext[3] + "w" if len(ext) > 3 else ext + "w"
    _createWorld(filename + "." + worldExt, imageSize)

  # Open Preview
  if preview:
    img.show()

def main():
  # Parse command line arguments
  parser = argparse.ArgumentParser()
  parser.add_argument('--package', dest='packageId', default=None, help='input: name of the package in database')
  parser.add_argument('--csv', dest='csvFile', default=None, help='input: CSV with list of tiles. Full tree required. Order of first 3 columns: x,y,z,...')
  parser.add_argument('--mbtiles', dest='mbTilesFile', default=None, help='input: name of the MBTiles file')
  parser.add_argument('--geojson', dest='geojson', default=None, help='input: geojson with polygons (in standard WGS84)')
  parser.add_argument('--tilemask', dest='tilemask', default=None, help='input: encoded tilemask')
  parser.add_argument('--image', dest='imageFile', default=None, help='output: target image (works with MBTiles input only)')
  parser.add_argument('--preview', action='store_true', default=False, help='open image Preview')
  parser.add_argument('--maxzoom', dest='maxZoom', type=int, default=TILEMASK_DEFAULT_MAX_ZOOM, help='maximum zoom (default %d)' % TILEMASK_DEFAULT_MAX_ZOOM)

  args = parser.parse_args()
  if args.mbTilesFile is not None:
    tileMask = processFileTilemask(args.mbTilesFile, args.maxZoom)
  elif args.csvFile is not None:
    tileMask = processFileCsv(args.csvFile, args.maxZoom)
  elif args.packageId is not None:
    tileMask = processDb(args.packageId, args.maxZoom)
  elif args.geojson is not None:
    tileMask = processPolygon(args.geojson, args.maxZoom)
  elif args.tilemask is not None:
    tileMask = args.tilemask
  else:
    sys.stderr.write("One of --package, --mbtiles, --csv, --geojson or --tilemask must be specified as input\n")
    sys.exit(-1)

  if args.imageFile is not None or args.preview:
    buildFileImage(tileMask, args.imageFile, args.preview, args.maxZoom)
  print(tileMask)

if __name__ == "__main__":
  main()
