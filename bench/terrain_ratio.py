# Vertical step tied to horizontal ground resolution.
#
# horizontal(z) at lat 45.5 = 40075017 * cos(lat) / 2**z / 512  m/px
# A schedule is defined by a single ratio R: vertical_step = horizontal(z) / R,
# snapped down to a power of two so terrarium's blue channel stays cheap.
#
# R = 1   -> "precision == m/px" literally
# R = 26.8 -> what mapterhorn ships (2**(19-z)/256)

import math
import os
import sqlite3
import subprocess
import sys

SRC = "bench/terrain/test.tif"
OUT = "bench/terrain"
LAT = 45.5
ZOOMS = list(range(13, 4, -1))


def horiz(z):
    return 40075016.686 * math.cos(math.radians(LAT)) / 2 ** z / 512


def pow2_snap(v):
    """nearest power of two, floored to keep the step no coarser than asked"""
    return 2.0 ** round(math.log2(v))


def schedule(ratio):
    return {z: pow2_snap(horiz(z) / ratio) for z in ZOOMS}


PLANS = {
    "R1": schedule(1),
    "R4": schedule(4),
    "R8": schedule(8),
    "R16": schedule(16),
    "R27_mapterhorn": schedule(26.8),
    # the plan C already chosen, for reference
    "C_chosen": {13: 1, 12: 1, 11: 2, 10: 4, 9: 8, 8: 16, 7: 32, 6: 64, 5: 128},
}


def build(name, sched):
    per_zoom = {}
    for z in ZOOMS:
        step = sched[z]
        path = f"{OUT}/ratio_{name}_z{z}.mbtiles"
        if os.path.exists(path):
            os.remove(path)
        subprocess.run(
            ["rio", "rgbify", "--format", "webp", "-j", "8", "-b", "-10000",
             "-i", str(step), "--round-digits", "0", "--encoding", "terrarium",
             "--max-z", str(z), "--min-z", str(z), SRC, path],
            check=True, capture_output=True,
        )
        with sqlite3.connect(path) as db:
            n, b = db.execute(
                "SELECT count(*), coalesce(sum(length(tile_data)),0) FROM tiles"
            ).fetchone()
        per_zoom[z] = b
    return per_zoom


def main():
    results = {}
    for name, sched in PLANS.items():
        pz = build(name, sched)
        results[name] = pz
        print(f"  {name}: {sum(pz.values())/1048576:.2f} MB", flush=True)

    print("\n=== steps (m) per zoom ===")
    print(f"{'PLAN':<16}" + "".join(f"{'z'+str(z):>9}" for z in ZOOMS))
    for name, sched in PLANS.items():
        print(f"{name:<16}" + "".join(f"{sched[z]:>9.2f}" for z in ZOOMS))

    print("\n=== test-window MB per zoom ===")
    print(f"{'PLAN':<16}" + "".join(f"{'z'+str(z):>9}" for z in ZOOMS) + f"{'TOTAL':>9}")
    for name in PLANS:
        pz = results[name]
        print(f"{name:<16}" + "".join(f"{pz[z]/1048576:>9.2f}" for z in ZOOMS)
              + f"{sum(pz.values())/1048576:>9.2f}")

    # scale onto the real production pyramid, per zoom, using plan C as the anchor
    REAL_A = {12: 91.8, 11: 30.8, 10: 9.2, 9: 2.8, 8: 0.9, 7: 0.4, 6: 0.2, 5: 0.1}
    WIN_A = {12: 22.15, 11: 6.37, 10: 1.58, 9: 0.38, 8: 0.09, 7: 0.03, 6: 0.01, 5: 0.004}
    # z13 has no production reference; scale it off the real z12 of the same plan
    print("\n=== scaled to real rhone-alpes (z5-z13) ===")
    a_tot = sum(REAL_A.values())
    print(f"{'PLAN':<16} {'MB':>8} {'vs A@z12':>10}")
    for name in PLANS:
        pz = results[name]
        tot = 0.0
        for z in ZOOMS:
            if z == 13:
                tot += REAL_A[12] * pz[13] / WIN_A[12]
            else:
                tot += REAL_A[z] * pz[z] / WIN_A[z]
        print(f"{name:<16} {tot:>8.1f} {100*(tot-a_tot)/a_tot:>+9.0f}%")
    print(f"{'A@z12 today':<16} {a_tot:>8.1f} {0:>+9.0f}%")


if __name__ == "__main__":
    main()
