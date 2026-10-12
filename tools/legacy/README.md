# Legacy pipeline

The shell and Python scripts Cairn replaced. They still run, and they are the reference that
Cairn's output is checked against — [docs/pipeline-reference.md](../../docs/pipeline-reference.md)
walks through them step by step.

You do not need any of this to use Cairn.

| Path | |
| --- | --- |
| `setup.sh` | builds the vendored tools (tippecanoe, planetiler, prime_server, Valhalla) and a Python venv |
| `env.sh` | puts them on `PATH`: `source tools/legacy/env.sh` |
| `requirements.txt` | the Python dependencies |
| `scripts/` | the pipeline itself: OSM download, terrain-RGB, contours, Valhalla package, tile filtering |
| `download_ign.sh` | builds a France-wide GeoTIFF from IGN RGE ALTI 5 m (needs GDAL) |
| `sources/` | the IGN RGE ALTI download list |

Run everything from the repository root. `env.sh` hard-codes a macOS `JAVA_HOME`; edit it for
your machine.
