# Recompress every tile blob in an mbtiles with a different gzip level.
# Pure container-level change: the decoded MVT bytes are byte-identical, only the
# deflate stream differs, so any client reads it unchanged.

import argparse
import gzip
import io
import shutil
import sqlite3
import zlib
from contextlib import closing


def regzip(blob, level):
  raw = gzip.decompress(blob)
  bos = io.BytesIO()
  with gzip.GzipFile(fileobj=bos, mode='wb', compresslevel=level, mtime=0) as f:
    f.write(raw)
  return bos.getvalue()


def main():
  parser = argparse.ArgumentParser()
  parser.add_argument(dest='source', help='input mbtiles')
  parser.add_argument(dest='output', help='output mbtiles')
  parser.add_argument('--level', dest='level', type=int, default=9, help='gzip level (1-9)')
  args = parser.parse_args()

  shutil.copyfile(args.source, args.output)

  with closing(sqlite3.connect(args.output)) as db:
    db.execute("PRAGMA journal_mode=OFF")
    db.execute("PRAGMA synchronous=OFF")
    cur = db.cursor()
    cur.execute("SELECT name FROM sqlite_master WHERE name='tiles_data'")
    compact = cur.fetchone() is not None

    if compact:
      cur.execute("SELECT tile_data_id, tile_data FROM tiles_data")
    else:
      cur.execute("SELECT rowid, tile_data FROM tiles")
    rows = cur.fetchall()

    before = after = 0
    updates = []
    for key, blob in rows:
      before += len(blob)
      out = regzip(blob, args.level)
      after += len(out)
      updates.append((out, key))

    if compact:
      cur.executemany("UPDATE tiles_data SET tile_data=? WHERE tile_data_id=?", updates)
    else:
      cur.executemany("UPDATE tiles SET tile_data=? WHERE rowid=?", updates)
    cur.close()
    db.commit()

  with closing(sqlite3.connect(args.output)) as db:
    db.execute("VACUUM")

  print("tiles       %d" % len(rows))
  print("blobs before %.1f MB" % (before / 1048576))
  print("blobs after  %.1f MB  (%.2f%%)" % (after / 1048576, 100.0 * (after - before) / before))


if __name__ == "__main__":
  main()
