# Benchmarks

The scripts that measured the option sets Cairn ships as presets (`measured`, `stock`). Each
`run_*.sh` builds a family of variants of one area and writes the archives and logs here
(git-ignored); `compare.sh` then prints a size ladder and a per-layer breakdown.

Run them from anywhere — they `cd` to the repository root — with the planetiler jar built:

```shell
tools/bench/run.sh        # the basemap ladder
tools/bench/compare.sh    # sizes, per layer
```

| Script | Sweeps |
| --- | --- |
| `run.sh`, `run_tol.sh`, `run_lc.sh`, `run_lc2.sh`, `run_iso.sh` | basemap flags: simplification tolerances, landcover merging |
| `run_routes.sh`, `run_routes2.sh` | route-layer flags |
| `run_mlt.sh` | MapLibre Tile (MLT) encoding flags |
| `terrain_*.py` | terrain-RGB encodings, pyramids and vertical-step schedules |

The `k*_symbols.json` files are route-symbol tables written by those runs.
