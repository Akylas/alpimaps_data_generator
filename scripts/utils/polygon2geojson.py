import re
import argparse
import os
import os.path
import json
from shapely.geometry import Polygon, mapping
import functools

def read_polygon(polygon_filename):
  with open(polygon_filename) as f:
    return f.readlines()

def clean_polygon(polygon_data):
  coordinates = polygon_data[2:][:-2]
  coordinates = [re.split(r'[\s\t]+', item) for item in coordinates]
  coordinates = [list(filter(None, item)) for item in coordinates]
  coordinates = functools.reduce(lambda a,b: a[-1].pop(0) and a if len(a[-1]) == 1 and a[-1][0] == 'END' else a.append(['END']) or a if b[0].startswith('END') else a[-1].append(b) or a, [[[]]] + coordinates)
  coordinates = [[(float(item[0]), float(item[1])) for item in coordgroup] for coordgroup in coordinates]
  return coordinates

def write_geojson(data, geojson_filename):
  if os.path.isfile(geojson_filename):
    os.remove(geojson_filename)

  with open(geojson_filename, 'w') as output:
    features = []
    for elem in data:
      features.append({'type':'Feature', 'geometry': mapping(Polygon(elem)), 'properties': {}})
    output.write(json.dumps({'type': 'FeatureCollection', 'features': features}))

def main(polygon_filename, geojson_filename):
  polygon_data = read_polygon(polygon_filename)
  coordinates = clean_polygon(polygon_data)
  write_geojson(coordinates, geojson_filename)

if __name__ == "__main__":
  parser = argparse.ArgumentParser()
  parser.add_argument("polygon_filename", help='output file name')
  args = parser.parse_args()

  geojson_filename = '.'.join(args.polygon_filename.split('.')[:-1]) + ".geojson"
  main(args.polygon_filename, geojson_filename)
