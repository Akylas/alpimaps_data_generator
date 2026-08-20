# Terrain RGB encoding sweep: mapbox vs terrarium at matched vertical precision.
#
# Each variant is (encoding, interval, round_digits). The effective vertical step is
# interval * 2**round_digits metres for BOTH encodings, so any size difference is the
# encoding itself, not precision.

import os
import subprocess
import sqlite3
import sys

SRC = "bench/terrain/test.tif"
OUT = "bench/terrain"

# name, encoding, interval, round_digits -> vertical step in metres
# IMPORTANT: client decode formulas are fixed.
#   mapbox    : -10000 + (R*65536 + G*256 + B) * 0.1   -> interval MUST stay 0.1
#   terrarium : (R*256 + G + B/256) - 32768            -> no parameters at all
# So for mapbox the only legal knob is round_digits (zeroing low bits), while for
# terrarium `interval` is just a data quantisation step and any value stays decodable.
VARIANTS = [
    ("mapbox_rd3_0.8m", "mapbox", 0.1, 3),
    ("mapbox_rd4_1.6m", "mapbox", 0.1, 4),
    ("mapbox_rd5_3.2m", "mapbox", 0.1, 5),
    ("terrarium_0.8m", "terrarium", 0.8, 0),
    ("terrarium_1.6m", "terrarium", 1.6, 0),
    ("terrarium_3.2m", "terrarium", 3.2, 0),
    ("terrarium_1m", "terrarium", 1.0, 0),
]


def size_of(path):
    with sqlite3.connect(path) as db:
        n, b = db.execute(
            "SELECT count(*), coalesce(sum(length(tile_data)),0) FROM tiles"
        ).fetchone()
    return n, b


def main():
    zoom = int(sys.argv[1]) if len(sys.argv) > 1 else 12
    rows = []
    for name, enc, iv, rd in VARIANTS:
        path = f"{OUT}/{name}_z{zoom}.mbtiles"
        if os.path.exists(path):
            os.remove(path)
        cmd = [
            "rio", "rgbify", "--format", "webp", "-j", "8",
            "-b", "-10000", "-i", str(iv), "--round-digits", str(rd),
            "--encoding", enc, "--max-z", str(zoom), "--min-z", str(zoom),
            SRC, path,
        ]
        subprocess.run(cmd, check=True, capture_output=True)
        n, b = size_of(path)
        rows.append((name, enc, iv * 2 ** rd, n, b))
        print(f"  built {name}: {n} tiles, {b/1048576:.2f} MB", flush=True)

    base = rows[0][4]
    print(f"\n=== z{zoom}, {rows[0][3]} tiles, 1x1 degree Alps window ===")
    print(f"{'VARIANT':<20} {'ENC':<10} {'STEP':>7} {'MB':>8} {'vs current':>11}")
    for name, enc, step, n, b in rows:
        print(f"{name:<20} {enc:<10} {step:>6.2f}m {b/1048576:>8.2f} {100*(b-base)/base:>+10.1f}%")


if __name__ == "__main__":
    main()
