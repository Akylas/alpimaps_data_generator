# Per-attribute-key byte accounting for an mbtiles of vector tiles.
# Minimal MVT protobuf reader - no external deps.
#
# For every attribute key it reports:
#   value_bytes - the shared per-tile value pool entries only that key points at
#   tag_bytes   - the per-feature (key_index, value_index) varint pairs
#   key_bytes   - the key name itself, once per tile
# so you can see what dropping a key would actually buy you.

import argparse
import gzip
import sqlite3
from collections import defaultdict
from contextlib import closing


def read_varint(buf, i):
  shift = result = 0
  while True:
    b = buf[i]
    i += 1
    result |= (b & 0x7F) << shift
    if not b & 0x80:
      return result, i
    shift += 7


def varint_len(n):
  c = 1
  while n >= 0x80:
    n >>= 7
    c += 1
  return c


def fields(buf, start, end):
  """Yield (field_number, wire_type, payload_start, payload_end, value)."""
  i = start
  while i < end:
    key, i = read_varint(buf, i)
    fn, wt = key >> 3, key & 7
    if wt == 0:
      v, j = read_varint(buf, i)
      yield fn, wt, i, j, v
      i = j
    elif wt == 2:
      ln, j = read_varint(buf, i)
      yield fn, wt, j, j + ln, None
      i = j + ln
    elif wt == 5:
      yield fn, wt, i, i + 4, None
      i += 4
    elif wt == 1:
      yield fn, wt, i, i + 8, None
      i += 8
    else:
      raise ValueError("bad wire type %d" % wt)


def scan_tile(blob, key_bytes, value_bytes, tag_bytes, feature_count):
  for fn, wt, s, e, _ in fields(blob, 0, len(blob)):
    if fn != 3:  # Tile.layers
      continue
    keys, val_sizes = [], []
    feats = []
    for lfn, lwt, ls, le, _ in fields(blob, s, e):
      if lfn == 3:  # Layer.keys
        keys.append(blob[ls:le].decode('utf-8', 'replace'))
        key_bytes[keys[-1]] += (le - ls) + varint_len(le - ls) + 1
      elif lfn == 4:  # Layer.values
        val_sizes.append((le - ls) + varint_len(le - ls) + 1)
      elif lfn == 2:  # Layer.features
        feats.append((ls, le))

    # a value-pool entry is stored once per tile no matter how many features point at it,
    # so charge it to its key once per tile - not once per reference
    seen_pairs = set()
    for fs, fe in feats:
      for ffn, ffwt, fps, fpe, _ in fields(blob, fs, fe):
        if ffn != 2:  # Feature.tags (packed)
          continue
        i = fps
        while i < fpe:
          ki, i = read_varint(blob, i)
          vi, i = read_varint(blob, i)
          if ki >= len(keys):
            continue
          k = keys[ki]
          tag_bytes[k] += varint_len(ki) + varint_len(vi)
          if vi < len(val_sizes) and (ki, vi) not in seen_pairs:
            seen_pairs.add((ki, vi))
            value_bytes[k] += val_sizes[vi]
      feature_count[0] += 1


def main():
  parser = argparse.ArgumentParser()
  parser.add_argument(dest='mbtiles')
  parser.add_argument('--zoom', dest='zoom', type=int, default=None, help='restrict to one zoom')
  args = parser.parse_args()

  key_bytes = defaultdict(int)
  value_bytes = defaultdict(int)
  tag_bytes = defaultdict(int)
  feature_count = [0]

  with closing(sqlite3.connect(args.mbtiles)) as db:
    if args.zoom is None:
      rows = db.execute("SELECT tile_data FROM tiles")
    else:
      rows = db.execute("SELECT tile_data FROM tiles WHERE zoom_level=?", (args.zoom,))
    n = 0
    for (blob,) in rows:
      if blob[:2] == b'\x1f\x8b':
        blob = gzip.decompress(blob)
      scan_tile(blob, key_bytes, value_bytes, tag_bytes, feature_count)
      n += 1

  total = sum(key_bytes.values()) + sum(value_bytes.values()) + sum(tag_bytes.values())
  print("tiles %d  features %d  attribute bytes %.2f MB" % (n, feature_count[0], total / 1048576))
  print("%-12s %10s %10s %10s %10s %7s" % ("KEY", "TOTAL_KB", "values", "tags", "keyname", "share"))
  rank = sorted(key_bytes, key=lambda k: -(key_bytes[k] + value_bytes[k] + tag_bytes[k]))
  for k in rank:
    t = key_bytes[k] + value_bytes[k] + tag_bytes[k]
    print("%-12s %10.1f %10.1f %10.1f %10.1f %6.1f%%" % (
      k, t / 1024, value_bytes[k] / 1024, tag_bytes[k] / 1024, key_bytes[k] / 1024, 100.0 * t / total))


if __name__ == "__main__":
  main()
