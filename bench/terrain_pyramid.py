# Full z5-z12 pyramid comparison for the terrain RGB tiles, mirroring what
# build_hillshades.sh produces today.
#
# Client decode formulas are FIXED, so the legal knobs differ per encoding:
#   mapbox    : -10000 + (R*65536 + G*256 + B) * 0.1  -> only round_digits may change
#   terrarium : (R*256 + G + B/256) - 32768           -> any data quantisation works
#
# Terrarium is only efficient when the quantisation step is a whole number of metres
# (or 1/2, 1/4 ...): then the blue channel is constant and costs nothing.

import os
import sqlite3
import subprocess

SRC = "bench/terrain/test.tif"
OUT = "bench/terrain"
ZOOMS = list(range(12, 4, -1))  # 12 down to 5

# current build_hillshades.sh: -r 3 at maxzoom, +1 per zoom down, --max-round-digits 7
CURRENT_RD = {12: 3, 11: 4, 10: 5, 9: 6, 8: 7, 7: 7, 6: 7, 5: 7}

# mapterhorn-style: vertical step halves with every zoom in, powers of two only
MAPTERHORN = {12: 0.5, 11: 1, 10: 2, 9: 4, 8: 8, 7: 16, 6: 32, 5: 64}

# one stop coarser everywhere - 1 m at maxzoom
METRE = {12: 1, 11: 2, 10: 4, 9: 8, 8: 16, 7: 32, 6: 64, 5: 128}

PLANS = [
    ("A_current_mapbox", "mapbox", {z: (0.1, CURRENT_RD[z]) for z in ZOOMS}),
    ("B_terrarium_mapterhorn", "terrarium", {z: (MAPTERHORN[z], 0) for z in ZOOMS}),
    ("C_terrarium_metre", "terrarium", {z: (METRE[z], 0) for z in ZOOMS}),
]


def build(name, enc, sched):
    total, tiles, per_zoom = 0, 0, {}
    for z in ZOOMS:
        iv, rd = sched[z]
        path = f"{OUT}/{name}_z{z}.mbtiles"
        if os.path.exists(path):
            os.remove(path)
        subprocess.run(
            ["rio", "rgbify", "--format", "webp", "-j", "8", "-b", "-10000",
             "-i", str(iv), "--round-digits", str(rd), "--encoding", enc,
             "--max-z", str(z), "--min-z", str(z), SRC, path],
            check=True, capture_output=True,
        )
        with sqlite3.connect(path) as db:
            n, b = db.execute(
                "SELECT count(*), coalesce(sum(length(tile_data)),0) FROM tiles"
            ).fetchone()
        per_zoom[z] = b
        total += b
        tiles += n
    return total, tiles, per_zoom


def main():
    results = []
    for name, enc, sched in PLANS:
        total, tiles, per_zoom = build(name, enc, sched)
        results.append((name, enc, sched, total, tiles, per_zoom))
        print(f"  {name}: {tiles} tiles, {total/1048576:.2f} MB", flush=True)

    base = results[0][3]
    print(f"\n=== z5-z12 pyramid, 1x1 degree Alps window ===")
    print(f"{'PLAN':<24} {'ENC':<10} {'MB':>8} {'vs current':>11}")
    for name, enc, sched, total, tiles, pz in results:
        print(f"{name:<24} {enc:<10} {total/1048576:>8.2f} {100*(total-base)/base:>+10.1f}%")

    print(f"\n{'ZOOM':<6} {'step A':>8} {'MB A':>8} {'step B':>8} {'MB B':>8} {'step C':>8} {'MB C':>8}")
    for z in ZOOMS:
        a, b, c = results[0], results[1], results[2]
        sa = a[2][z][0] * 2 ** a[2][z][1]
        print(f"z{z:<5} {sa:>7.2f}m {a[5][z]/1048576:>8.2f} "
              f"{b[2][z][0]:>7.2f}m {b[5][z]/1048576:>8.2f} "
              f"{c[2][z][0]:>7.2f}m {c[5][z]/1048576:>8.2f}")


if __name__ == "__main__":
    main()
