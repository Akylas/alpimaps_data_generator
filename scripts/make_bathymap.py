#!/usr/bin/env python3

"""
make_bathymap.py

Builds ONE vector MBTiles holding the low-zoom half of the map:

    land        the coastline fill, so z0-z7 is never a transparent void
    landcover   ESA WorldCover classes, cumulative across zooms
    bathymetry  Natural Earth depth bands, class 0..11

It is meant to be rendered from z0 until the main (planetiler) basemap
fades in around z8/z9.

Why it is built the way it is
-----------------------------

*Land first.* The VersaTiles landcover container carries landcover patches
and nothing else. Painted on their own they cover ~96% of a z6 tile but only
~62% of the same ground at z7, so the map visibly falls apart as you zoom:
big blocks of colour become confetti on a transparent background. The `land`
layer (Natural Earth 10m land + minor islands) is the floor under all of it.

*Landcover is cumulative.* Each zoom is built as `detail(z)` unioned with
whatever the previous zoom covered and this zoom does not:

    result(z) = detail(z) + (result(z-1) - detail(z))

So coverage never drops as you zoom in - it only gains detail. That is what
removes the z6 -> z7 cliff: France stays filled, and cities (`built`) and
finer class boundaries appear on top.

*Chunked.* Dissolving a whole planet of z7 polygons in one shapely call is
not survivable. Work is done per chunk - one tile of `--chunk-zoom` (default
z3, so at most 8x8 = 64 chunks for a global build). Source tiles are read
with `CLIP=NO`, so each tile carries its buffer and the union stitches
across tile seams inside a chunk; chunk results are cut on the exact chunk
boundary, so neighbouring chunks share coincident edges.

*Classes do not overlap.* Classes are resolved in a fixed priority order
(built wins over vegetation, sand is the filler), each one subtracting the
classes above it. Overlapping fills z-fight in the renderer.

Caching
-------

Nothing that costs a download is keyed on the pipeline version:

    <cache>/natural-earth/            zips + extracted shapefiles, forever
    <cache>/landcover/<region>/       the regional VersaTiles extract, forever
    <cache>/landcover/<region>/mvt/   uncompressed MVT tiles, keyed on the
                                      extract's size+mtime
    <cache>/work/<region>/v<N>/       everything derived, keyed on version N

`<region>` is a hash of the bbox and zoom range only. `--force` rebuilds the
derived products and never touches a download; `--refresh-source` is the only
flag that re-fetches the regional extract.

Dependencies
------------

Python:  geopandas, shapely, pyogrio, requests
CLI:     versatiles, tippecanoe

Example:

    python scripts/make_bathymap.py \
        --bbox -10 30 40 48 \
        --output output/mediterranean.mbtiles
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import math
import multiprocessing
import os
import shutil
import subprocess
import sys
import time
import zipfile
from pathlib import Path
from typing import Any, Callable, Iterable, Iterator

import geopandas as gpd
import pandas as pd
import requests
import shapely
from shapely.geometry import box
from shapely.ops import unary_union


# ============================================================================
# CONFIGURATION
# ============================================================================

# Bump only when the *derived* products change. Downloads are never keyed on
# it, so a bump costs CPU and no bandwidth.
PIPELINE_VERSION = 24

DEFAULT_MIN_ZOOM = 0
DEFAULT_MAX_ZOOM = 7

# How many source tiles one chunk may hold at the maximum zoom before the grid
# is split finer. Every chunk boundary is a cut through the dissolved geometry,
# so the fewer chunks the better - a region small enough to process whole has
# no internal seams at all. This is the memory ceiling, not a tuning knob.
CHUNK_TILE_BUDGET = 1500

# Ceiling on how far the grid is refined to feed the workers.
MAX_CHUNKS_PER_JOB = 4

WGS84 = "EPSG:4326"
WEB_MERCATOR = "EPSG:3857"

# Half the Web Mercator square, in metres.
MERCATOR_EXTENT = 20037508.342789244
MERCATOR_MAX_LAT = 85.05112878

LANDCOVER_URL = "https://download.versatiles.org/landcover-vectors.versatiles"

# The layer inside the VersaTiles container that carries the classes. The
# container also has `water_polygons`, but only at z0-z3; glaciers come from
# Natural Earth instead, at 10m for every zoom.
LANDCOVER_LAYER = "land"
LANDCOVER_CLASS_FIELD = "kind"

# What the layers are called in the output MBTiles.
LANDCOVER_OUTPUT_LAYER = "global_landcover"

# Named for compatibility with Mapbox depth styles. `min_depth` is the isobath
# in metres, and it is the *minimum* depth found anywhere inside the polygon:
# the polygon carrying min_depth 200 is the 200 m contour, so every point in
# it is at least 200 m deep. Nested, so the deeper polygons sit inside it.
BATHY_OUTPUT_LAYER = "depth"
BATHY_PROPERTY = "min_depth"

# Mirrors, fastest first. The S3 bucket is the canonical home but regularly
# serves at a few KB/s, which is a twenty-minute stall on a 7 MB zip; the
# Nacis CDN carries the same files. Both support Range, so a stalled transfer
# resumes rather than restarting.
NATURAL_EARTH_MIRRORS = (
    "https://naciscdn.org/naturalearth/10m/physical",
    "https://naturalearth.s3.amazonaws.com/10m_physical",
)

DOWNLOAD_ATTEMPTS = 4

# name -> (archive stem, what it is used for)
NATURAL_EARTH_SETS = {
    "bathymetry": "ne_10m_bathymetry_all",
    "land": "ne_10m_land",
    "minor_islands": "ne_10m_minor_islands",
    "glaciated": "ne_10m_glaciated_areas",
    "ice_shelves": "ne_10m_antarctic_ice_shelves_polys",
    "playas": "ne_10m_playas",
}

# Extra tiles fetched around the bbox, so a dissolve near the edge is not
# starved of the geometry that continues past it.
LANDCOVER_BBOX_BORDER = 2

TIPPECANOE_BUFFER = 16

# Coordinate resolution inside a tile, as a power of two. Tippecanoe's default
# is 12, i.e. 4096 units across a tile - eight units per screen pixel at 512px,
# three bits of precision nobody can see. Measured on a global z0-7 build:
# 12 -> 115.3 MB, 11 -> 101.0 MB, 10 -> 87.0 MB.
#
# Independent of the generalisation pixel units below, which stay pegged to
# TILE_EXTENT so that tuning them does not shift when this changes.
DEFAULT_DETAIL = 11

# Coordinates are snapped to this grid (degrees) before being written, which
# keeps the GeoJSON small - ~0.1 m, far below a z7 pixel.
OUTPUT_PRECISION = 1e-6

# Tippecanoe's default tile detail: 4096 units across a tile.
TILE_EXTENT = 4096

# The smallest polygon worth keeping, in squared tile units.
#
# A tile is TILE_EXTENT units across but only 256-512 screen pixels, so one
# screen pixel is 8-16 units and a screen pixel of area is 64-256 units squared.
# 128 is therefore still below anything a viewer can see, and it is the
# difference between 41k invisible bathymetry fragments at z0 and ~2k that
# carry 99.7% of the area.
MIN_PIXELS = 128.0

# Landcover generalisation, in tile units at the zoom being built.
#
# Landcover arrives as thousands of ragged patches per tile. Drawn as they
# come, a z7 view of France is confetti. Two knobs turn that into zones:
# SMOOTH_PIXELS is the radius of a close-then-open pass, which welds patches
# that are within 2r of each other and shaves anything thinner than 2r;
# MIN_ZONE_PIXELS then drops what is still small. What gets dropped is not a
# hole - the cumulative fill puts the coarser zoom's zone underneath it.
SMOOTH_PIXELS = 12.0
MIN_ZONE_PIXELS = 4096.0

# Douglas-Peucker tolerance for landcover, applied to all classes at once.
#
# Per-class simplification is not safe - two classes that share a border would
# each move it somewhere different and open a sliver - so this goes through
# `shapely.coverage_simplify`, which simplifies a polygonal coverage and keeps
# the shared edges shared. Measured on real z7 chunks: 4 px keeps 100.00% of
# the area and 0.003% overlap for 75% of the vertices.
LANDCOVER_SIMPLIFY_PIXELS = 4.0

# A landmass that ends up with no landcover at all disappears completely, since
# there is no land layer underneath any more - that is how New Caledonia,
# Vanuatu and Fiji became open sea at low zoom. An island that would vanish
# keeps its dominant class instead, exempt from MIN_ZONE_PIXELS but still
# subject to being genuinely too small to draw.
ISLAND_RESCUE_MAX_ZONES = 100.0
ISLAND_FALLBACK_CLASS = "grass"

# Bathymetry generalisation, also in tile units at the zoom being built.
#
# Depth contours are a single nested stack, so they need no smoothing pass -
# a plain Douglas-Peucker at the zoom's own scale is enough, and because the
# contours are simplified *before* the bands are cut out of them, neighbouring
# bands still share their edge exactly.
BATHY_SIMPLIFY_PIXELS = 14.0

# Small on purpose. Smoothness comes from the simplification above; this only
# decides what is too small to draw at all, and a depth polygon is a real
# basin, not speckle. At 16384 the Mediterranean lost a fifth of its 3000 m
# contour at z2 and was down to two pieces of it. 1024 tile units squared is
# about a 4x4 screen pixel blob, recovers essentially all of that, and costs
# roughly 8% of the file.
BATHY_MIN_ZONE_PIXELS = 1024.0

BATHY_LEVELS = [0, 200, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000]

# Natural Earth files each depth as a letter-prefixed shapefile.
BATHY_LETTERS = {
    0: "L", 200: "K", 1000: "J", 2000: "I", 3000: "H", 4000: "G",
    5000: "F", 6000: "E", 7000: "D", 8000: "C", 9000: "B", 10000: "A",
}

# class id -> (shallow contour, deep contour); the deepest band is open-ended.
BATHY_CLASSES = [
    {"class": index, "min": BATHY_LEVELS[index],
     "max": BATHY_LEVELS[index + 1] if index + 1 < len(BATHY_LEVELS) else None}
    for index in range(len(BATHY_LEVELS))
]

# VersaTiles `kind` -> our class. Everything not listed is dropped.
LANDCOVER_CLASS_MAP = {
    "forest": "tree",
    "trees": "tree",
    "wood": "tree",
    "farmland": "crop",
    "cropland": "crop",
    "crop": "crop",
    "scrub": "scrub",
    "shrub": "scrub",
    "shrubland": "scrub",
    "grassland": "grass",
    "grass": "grass",
    "heath": "heath",
    "bare_rock": "sand",
    "bare-rock": "sand",
    "bare": "sand",
    "desert": "sand",
    "sand": "sand",
    "residential": "built",
    "urban": "built",
    "built": "built",
    "marsh": "marsh",
    "swamp": "swamp",
    "wetland": "marsh",
    "glacier": "glacier",
    "ice": "glacier",
    "snow": "glacier",
}

# Highest priority first. A class is cut out of every class below it, so the
# output has no overlaps: cities stay visible over their surroundings, and
# `sand` (bare ground) is only ever what nothing else claimed.
LANDCOVER_PRIORITY = [
    "built",
    "glacier",
    "swamp",
    "marsh",
    "tree",
    "crop",
    "scrub",
    "heath",
    "grass",
    "sand",
]


# ============================================================================
# LOGGING
# ============================================================================

_START = time.monotonic()


def log(message: str) -> None:
    print(f"[bathymap] {message}", flush=True)


def stage(name: str) -> None:
    """A stage banner, in a shape a supervising process can key on."""
    elapsed = time.monotonic() - _START
    print(f"[bathymap] STAGE {name} t={elapsed:.1f}s", flush=True)


def progress(name: str, done: int, total: int) -> None:
    percent = 100 * done / total if total else 100.0
    print(f"[bathymap] PROGRESS {name} {done}/{total} {percent:.0f}%", flush=True)


class BathymapError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise BathymapError(message)


# ============================================================================
# TOOLS
# ============================================================================

REPO_ROOT = Path(__file__).resolve().parent.parent


def resolve_tool(name: str, explicit: str | None) -> str:
    """
    Find an executable, preferring what the caller asked for.

    Looked at in order: the explicit path or name, `PATH`, then the two places
    this repository builds or installs its own copies. A packaged Cairn build
    passes its bundled binary explicitly and never reaches the fallbacks.
    """

    if explicit:
        candidate = Path(explicit)
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate.resolve())
        found = shutil.which(explicit)
        if found:
            return found
        fail(f"'{explicit}' is not an executable and is not on PATH")

    found = shutil.which(name)
    if found:
        return found

    for candidate in (
        REPO_ROOT / name / name,
        REPO_ROOT / "venv" / "bin" / name,
        Path(sys.prefix) / "bin" / name,
    ):
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate.resolve())

    fail(
        f"required command '{name}' was not found. Put it on PATH "
        f"(`source env.sh` covers tippecanoe) or pass --{name}"
    )
    raise AssertionError("unreachable")


def run(command: list[str]) -> None:
    log("$ " + " ".join(str(part) for part in command))
    result = subprocess.run([str(part) for part in command])
    if result.returncode != 0:
        fail(f"command failed with exit code {result.returncode}: {command[0]}")


# ============================================================================
# DOWNLOAD
# ============================================================================

def fetch(url: str, partial: Path) -> None:
    """One transfer attempt, appending to `partial` if there is something to resume."""

    headers: dict[str, str] = {}

    if partial.exists() and partial.stat().st_size > 0:
        offset = partial.stat().st_size
        headers["Range"] = f"bytes={offset}-"
        log(f"resuming at {offset:,} bytes: {url}")
    else:
        log(f"download: {url}")

    with requests.get(url, headers=headers, stream=True, timeout=(30, 120)) as response:
        # A server that ignores Range restarts the file; appending to the old
        # bytes would silently produce a corrupt archive.
        if response.status_code == 200 and partial.exists():
            partial.unlink()

        response.raise_for_status()
        mode = "ab" if response.status_code == 206 else "wb"

        with open(partial, mode) as handle:
            for chunk in response.iter_content(chunk_size=1024 * 1024):
                if chunk:
                    handle.write(chunk)


def download(urls: str | list[str], destination: Path) -> None:
    """
    Fetch once, from whichever mirror answers.

    Mirrors are tried in order and each is retried, because a stalled transfer
    is the normal failure here rather than a missing file. Bytes already in the
    `.part` file are kept across every attempt.
    """

    if isinstance(urls, str):
        urls = [urls]

    destination.parent.mkdir(parents=True, exist_ok=True)

    if destination.exists() and destination.stat().st_size > 0:
        log(f"cached: {destination.name}")
        return

    partial = destination.with_name(destination.name + ".part")
    last: Exception | None = None

    for attempt in range(DOWNLOAD_ATTEMPTS):
        for url in urls:
            try:
                fetch(url, partial)
                partial.replace(destination)
                return
            except Exception as error:
                last = error
                log(f"download failed ({type(error).__name__}): {url}")

        if attempt + 1 < DOWNLOAD_ATTEMPTS:
            delay = 2 ** attempt
            log(f"retrying in {delay}s")
            time.sleep(delay)

    fail(f"could not download {destination.name}: {last}")


def download_natural_earth(cache: Path) -> dict[str, Path]:
    """Download and extract every Natural Earth set. Cached across versions."""

    root = cache / "natural-earth"
    directories: dict[str, Path] = {}

    for name, stem in NATURAL_EARTH_SETS.items():
        archive = root / f"{stem}.zip"
        directory = root / name

        download([f"{base}/{stem}.zip" for base in NATURAL_EARTH_MIRRORS], archive)

        # Keyed on the archive, not on the pipeline version: a version bump
        # must not re-extract 35 MB of shapefiles.
        marker = directory / ".extracted"
        signature = f"{archive.stat().st_size}"

        if marker.exists() and marker.read_text(encoding="utf-8").strip() == signature:
            directories[name] = directory
            continue

        log(f"extracting {stem}")
        directory.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(archive) as zip_file:
            zip_file.extractall(directory)
        marker.write_text(signature, encoding="utf-8")

        directories[name] = directory

    return directories


# ============================================================================
# BBOX AND TILE MATH
# ============================================================================

Bbox = tuple[float, float, float, float]


def validate_bbox(bbox: Bbox) -> None:
    west, south, east, north = bbox
    if not -180 <= west < east <= 180:
        fail("invalid longitude bounds: west must be < east and both within -180..180")
    if not -MERCATOR_MAX_LAT <= south < north <= MERCATOR_MAX_LAT:
        fail(f"invalid latitude bounds: must be within +-{MERCATOR_MAX_LAT}")


def bbox_string(bbox: Bbox) -> str:
    return ",".join(f"{value:.8f}" for value in bbox)


def is_global(bbox: Bbox) -> bool:
    return (
        bbox[0] <= -179.999
        and bbox[2] >= 179.999
        and bbox[1] <= -MERCATOR_MAX_LAT + 1e-6
        and bbox[3] >= MERCATOR_MAX_LAT - 1e-6
    )


def region_key(bbox: Bbox, min_zoom: int, max_zoom: int) -> str:
    """
    Identity of a downloaded region.

    Deliberately free of PIPELINE_VERSION: the whole point is that changing
    how tiles are processed does not re-download them.
    """

    text = f"{bbox_string(bbox)}-z{min_zoom}-{max_zoom}-b{LANDCOVER_BBOX_BORDER}"
    return hashlib.sha1(text.encode("utf-8")).hexdigest()[:12]


def lon_to_tile_x(lon: float, zoom: int) -> float:
    return (lon + 180.0) / 360.0 * (2 ** zoom)


def lat_to_tile_y(lat: float, zoom: int) -> float:
    lat = max(-MERCATOR_MAX_LAT, min(MERCATOR_MAX_LAT, lat))
    radians = math.radians(lat)
    y = (1.0 - math.log(math.tan(radians) + 1.0 / math.cos(radians)) / math.pi) / 2.0
    return y * (2 ** zoom)


def tile_bounds_3857(zoom: int, x: int, y: int) -> tuple[float, float, float, float]:
    size = 2.0 * MERCATOR_EXTENT / (2 ** zoom)
    left = -MERCATOR_EXTENT + x * size
    top = MERCATOR_EXTENT - y * size
    return (left, top - size, left + size, top)


def tiles_covering(bbox: Bbox, zoom: int) -> list[tuple[int, int]]:
    """Every tile of `zoom` that the bbox touches."""

    span = 2 ** zoom
    x0 = max(0, min(span - 1, int(math.floor(lon_to_tile_x(bbox[0], zoom)))))
    x1 = max(0, min(span - 1, int(math.ceil(lon_to_tile_x(bbox[2], zoom))) - 1))
    y0 = max(0, min(span - 1, int(math.floor(lat_to_tile_y(bbox[3], zoom)))))
    y1 = max(0, min(span - 1, int(math.ceil(lat_to_tile_y(bbox[1], zoom))) - 1))

    return [(x, y) for x in range(x0, x1 + 1) for y in range(y0, y1 + 1)]


def choose_chunk_zoom(bbox: Bbox, max_zoom: int, jobs: int,
                      explicit: int | None) -> int:
    """
    The processing grid: coarse enough to fit in memory, fine enough to fill
    the workers.

    Chunking does two jobs and they pull against each other. It bounds memory -
    a chunk is dissolved as one piece - and it is the unit of parallelism.
    Against that, every chunk boundary cuts geometry that was continuous, which
    shows up as an extra polygon edge.

    That edge is only an edge. Measured on a global build, coverage across a
    chunk boundary is 100%, identical to a control strip away from one: no gap,
    no sliver, nothing a fill can show. It is visible only in a style that
    strokes polygon outlines - which also draws every MVT tile boundary, since
    vector tiles clip their features regardless. So the grid follows the job
    count, and `--chunk-zoom 0` is there for anyone who wants one seamless
    chunk and no parallelism.
    """

    if explicit is not None:
        return explicit

    tiles = tiles_covering(bbox, max_zoom)

    smallest = max_zoom
    for chunk_zoom in range(0, max_zoom + 1):
        factor = 2 ** (max_zoom - chunk_zoom)
        worst = 0
        counts: dict[tuple[int, int], int] = {}
        for x, y in tiles:
            key = (x // factor, y // factor)
            counts[key] = counts.get(key, 0) + 1
        worst = max(counts.values()) if counts else 0

        if worst <= CHUNK_TILE_BUDGET:
            smallest = chunk_zoom
            break

    # Then go finer, aiming for a few chunks per worker so the uneven ones -
    # an all-ocean chunk costs nothing, a chunk of Europe costs minutes - even
    # out. Stop one zoom short of the maximum: below that a chunk holds a
    # handful of source tiles and the boundary work outweighs the parallelism.
    target = MAX_CHUNKS_PER_JOB * jobs
    finest = max(smallest, max_zoom - 1)

    for chunk_zoom in range(smallest, finest + 1):
        if len(tiles_covering(bbox, chunk_zoom)) >= target:
            return chunk_zoom

    return finest


def child_tiles(zoom: int, chunk_zoom: int, cx: int, cy: int) -> list[tuple[int, int]]:
    """The `zoom` tiles inside chunk (cx, cy), or the ancestor if zoom is coarser."""

    if zoom >= chunk_zoom:
        factor = 2 ** (zoom - chunk_zoom)
        return [
            (cx * factor + i, cy * factor + j)
            for i in range(factor)
            for j in range(factor)
        ]

    factor = 2 ** (chunk_zoom - zoom)
    return [(cx // factor, cy // factor)]


# ============================================================================
# GEOMETRY HELPERS
# ============================================================================

def make_valid(geometry: Any) -> Any:
    if geometry is None or geometry.is_empty:
        return None
    if not geometry.is_valid:
        geometry = shapely.make_valid(geometry)
    return None if geometry.is_empty else geometry


def polygon_only(geometry: Any) -> Any:
    """
    Keep the polygonal part of a geometry.

    Unions of noisy source data routinely come back as GeometryCollections
    with stray lines; feeding those to tippecanoe as a polygon layer is how
    a class silently disappears.
    """

    geometry = make_valid(geometry)
    if geometry is None:
        return None

    if geometry.geom_type in ("Polygon", "MultiPolygon"):
        return geometry

    if geometry.geom_type == "GeometryCollection":
        parts = [
            part
            for part in geometry.geoms
            if part.geom_type in ("Polygon", "MultiPolygon")
        ]
        if parts:
            return make_valid(unary_union(parts))

    return None


def dissolve(geometries: Iterable[Any]) -> Any:
    parts = [g for g in (make_valid(g) for g in geometries) if g is not None]
    if not parts:
        return None
    return polygon_only(shapely.union_all(parts))


def zoom_grid(zoom: int) -> float:
    """The size of one tile pixel at `zoom`, in Web Mercator metres."""
    return 2.0 * MERCATOR_EXTENT / (2 ** zoom * TILE_EXTENT)


def reduce_for_zoom(geometry: Any, zoom: int, *, simplify_pixels: float = 0.0,
                    min_pixels: float = MIN_PIXELS) -> Any:
    """
    Cut a geometry down to what the zoom can actually draw.

    Snapping to the zoom's pixel grid is the part that is always safe: two
    geometries sharing a border snap their shared vertices identically, so the
    border stays shared. Simplifying does not have that guarantee, so
    `simplify_pixels` defaults to off - it is correct for bathymetry, whose
    bands are all cut from the same already-simplified contours, and wrong for
    anything whose pieces are simplified independently and then expected to
    still abut.

    This is what keeps a whole planet inside tippecanoe's tile size limit at
    z0. Snapping alone took the 0 m contour from 412k coordinates to 137k;
    simplifying at one pixel first takes it to 38k.
    """

    grid = zoom_grid(zoom)

    try:
        if simplify_pixels > 0:
            geometry = shapely.simplify(
                geometry, grid * simplify_pixels, preserve_topology=True
            )
        geometry = polygon_only(shapely.set_precision(geometry, grid))
    except Exception:
        geometry = polygon_only(geometry)

    if geometry is None:
        return None

    return drop_small(geometry, grid * grid * min_pixels)


def drop_small(geometry: Any, minimum: float) -> Any:
    """Keep only the parts, and only the holes, that are worth drawing."""

    kept = []
    for part in geometry_parts(geometry):
        if part.area < minimum:
            continue
        holes = [ring for ring in part.interiors
                 if shapely.Polygon(ring).area >= minimum]
        kept.append(shapely.Polygon(part.exterior, holes)
                    if len(holes) != len(part.interiors) else part)

    if not kept:
        return None
    return kept[0] if len(kept) == 1 else shapely.multipolygons(kept)


def generalize(geometry: Any, zoom: int, smooth_pixels: float,
               min_zone_pixels: float) -> Any:
    """
    Turn a class's patchwork into zones.

    A close (dilate then erode) welds neighbouring patches and swallows the
    pinholes between them; the open (erode then dilate) that follows shaves
    off the tendrils and specks the close just created. Then anything still
    below `min_zone_pixels` goes.

    `quad_segs` is deliberately low: this is a smoothing pass, and round caps
    faithful to a circle would triple the vertex count for no visible gain.
    """

    grid = zoom_grid(zoom)

    if smooth_pixels > 0:
        radius = smooth_pixels * grid
        try:
            smoothed = geometry.buffer(radius, quad_segs=2)
            smoothed = smoothed.buffer(-2.0 * radius, quad_segs=2)
            smoothed = smoothed.buffer(radius, quad_segs=2)
            smoothed = polygon_only(smoothed)
            if smoothed is not None:
                geometry = smoothed
        except Exception:
            pass

    geometry = polygon_only(geometry)
    if geometry is None:
        return None

    return drop_small(geometry, min_zone_pixels * grid * grid)


# Grid sizes, in Web Mercator metres, to retry a failed overlay on. GEOS
# raises "side location conflict" on geometry that survives make_valid but
# still has coordinates too close together to intersect exactly; snapping both
# operands to a coarse-enough grid is the documented way out. A millimetre is
# already far below anything this pipeline can render.
OVERLAY_GRID_SIZES = (0.0, 0.001, 0.01, 0.1, 1.0)


def snap_clean(geometry: Any) -> Any:
    """
    Best-effort repair by snapping to a grid.

    `set_precision` is itself an overlay and throws the same topology error it
    is being used to prevent, so every grid size is a try, and a geometry that
    refuses all of them is returned as it came - `safe_intersection` can still
    cope with it.
    """

    for grid_size in OVERLAY_GRID_SIZES[1:]:
        try:
            cleaned = polygon_only(shapely.set_precision(geometry, grid_size))
        except Exception:
            continue
        if cleaned is not None:
            return cleaned

    return geometry


def safe_intersection(a: Any, b: Any) -> Any:
    """
    Intersect two geometries, giving GEOS progressively more slack.

    A world-scale coastline union is exactly the input that trips the exact
    overlay: 6700 polygons, reprojected, some of them touching at a single
    point. Failing the whole build over it is not an option, and neither is
    silently returning nothing.
    """

    last: Exception | None = None

    for grid_size in OVERLAY_GRID_SIZES:
        for left, right in ((a, b), (make_valid(a), make_valid(b))):
            if left is None or right is None:
                continue
            try:
                return shapely.intersection(
                    left, right, grid_size=grid_size or None
                )
            except Exception as error:
                last = error

    raise BathymapError(f"could not intersect geometry: {last}")


def simplify_coverage(classes: dict[str, Any], zoom: int,
                      simplify_pixels: float) -> dict[str, Any]:
    """
    Douglas-Peucker every class together, so shared borders stay shared.

    `coverage_simplify` is the only way to do this safely. Simplifying each
    class on its own moves the border it shares with its neighbour to two
    different places and leaves a sliver between them.
    """

    if simplify_pixels <= 0 or not classes:
        return classes

    names = list(classes)
    try:
        simplified = shapely.coverage_simplify(
            [classes[name] for name in names],
            zoom_grid(zoom) * simplify_pixels,
            simplify_boundary=True,
        )
    except Exception:
        return classes

    out: dict[str, Any] = {}
    for name, geometry in zip(names, simplified):
        kept = polygon_only(geometry)
        if kept is not None:
            out[name] = kept
    return out or classes


def rescue_islands(result: dict[str, Any], window: Any, detail: dict[str, Any],
                   minimum: float) -> dict[str, Any]:
    """
    Put back any landmass that generalisation erased entirely.

    Only whole components of the window are considered, and only small ones -
    a continent is never wholly uncovered, so testing it would be wasted work.
    A rescued island takes the class that covered most of it before the
    thresholds were applied.
    """

    covered = dissolve(result.values()) if result else None
    rescued = dict(result)

    for island in geometry_parts(window):
        if island.area < minimum or island.area > minimum * ISLAND_RESCUE_MAX_ZONES:
            continue
        if covered is not None and island.intersection(covered).area > 0.05 * island.area:
            continue

        best, best_area = ISLAND_FALLBACK_CLASS, 0.0
        for name, geometry in detail.items():
            try:
                overlap = island.intersection(geometry).area
            except Exception:
                continue
            if overlap > best_area:
                best, best_area = name, overlap

        rescued[best] = (
            island if best not in rescued
            else dissolve([rescued[best], island])
        )

    return rescued


def clip(geometry: Any, window: Any) -> Any:
    if geometry is None:
        return None
    return polygon_only(safe_intersection(geometry, window))


def load_shapefile(directory: Path, stem_hint: str, bbox: Bbox) -> Any:
    """Union every polygon of a Natural Earth set that touches the bbox."""

    matches = sorted(directory.rglob(f"{stem_hint}*.shp"))
    if not matches:
        fail(f"cannot find a shapefile matching '{stem_hint}*.shp' under {directory}")

    window = box(*bbox)
    parts = []

    for path in matches:
        frame = gpd.read_file(path, bbox=None if is_global(bbox) else bbox)
        if frame.empty:
            continue
        frame = frame.set_crs(WGS84) if frame.crs is None else frame.to_crs(WGS84)
        for geometry in frame.geometry:
            geometry = clip(geometry, window) if not is_global(bbox) else polygon_only(geometry)
            if geometry is not None:
                parts.append(geometry)

    return dissolve(parts)


# ============================================================================
# GEOJSON SEQUENCE OUTPUT
# ============================================================================

class FeatureWriter:
    """
    Streaming newline-delimited GeoJSON.

    tippecanoe reads this format directly, so a planet-sized layer never has
    to exist as one JSON document in memory.
    """

    def __init__(self, path: Path):
        path.parent.mkdir(parents=True, exist_ok=True)
        self._path = path
        self._partial = path.with_name(path.name + ".partial")
        self._handle = open(self._partial, "w", encoding="utf-8")
        self.count = 0

    def write(self, geometry: Any, properties: dict[str, Any],
              minzoom: int | None = None, maxzoom: int | None = None) -> None:
        geometry = polygon_only(geometry)
        if geometry is None:
            return

        try:
            reduced = shapely.set_precision(geometry, OUTPUT_PRECISION)
            geometry = polygon_only(reduced) or geometry
        except Exception:
            # Precision reduction is an optimisation; never lose a feature to it.
            pass

        head: dict[str, Any] = {"type": "Feature"}
        if minzoom is not None or maxzoom is not None:
            zooms: dict[str, int] = {}
            if minzoom is not None:
                zooms["minzoom"] = minzoom
            if maxzoom is not None:
                zooms["maxzoom"] = maxzoom
            head["tippecanoe"] = zooms
        head["properties"] = properties

        prefix = json.dumps(head, separators=(",", ":"))[:-1]
        self._handle.write(prefix + ',"geometry":' + shapely.to_geojson(geometry) + "}\n")
        self.count += 1

    def close(self) -> Path:
        self._handle.close()
        self._partial.replace(self._path)
        return self._path

    def __enter__(self) -> "FeatureWriter":
        return self

    def __exit__(self, *_exc: Any) -> None:
        if not self._handle.closed:
            self._handle.close()
            self._partial.unlink(missing_ok=True)


def cached(path: Path, force: bool, what: str,
           inputs: Iterable[Path] = ()) -> bool:
    """
    Whether `path` can be reused.

    `inputs` guards against the trap that a rebuilt intermediate leaves a
    stale final product behind: the MBTiles existed, so it was kept, and a
    whole run's work went into a file nobody read. Anything older than what it
    was built from is not a cache hit.
    """

    if force or not path.exists() or path.stat().st_size == 0:
        return False

    mine = path.stat().st_mtime
    for source in inputs:
        if source.exists() and source.stat().st_mtime > mine:
            log(f"stale {what}: {source.name} is newer")
            return False

    log(f"cached {what}: {path}")
    return True


# ============================================================================
# LAND
# ============================================================================

def load_land_mask(directories: dict[str, Path], bbox: Bbox) -> Any:
    """
    The coastline, as a clip mask rather than a layer.

    Landcover is not clean at the coast. The cumulative fill inherits the
    coarse low zooms, whose shoreline cuts straight across bays, and at z7
    that inherited edge sits tens of kilometres out to sea - the long
    diagonals that ran through the Bay of Biscay and the Galician coast.
    Cutting every class against a real 10m coastline removes them.

    Antarctic ice shelves and glaciers are unioned in, because they are
    legitimately not land and would otherwise be clipped away.
    """

    stage("land_mask")

    parts = []
    for name, hint in (
        ("land", "ne_10m_land"),
        ("minor_islands", "ne_10m_minor_islands"),
        ("glaciated", "ne_10m_glaciated_areas"),
        ("ice_shelves", "ne_10m_antarctic_ice_shelves_polys"),
    ):
        geometry = load_shapefile(directories[name], hint, bbox)
        if geometry is not None:
            parts.append(geometry)

    merged = dissolve(parts)
    if merged is None:
        fail("no land geometry in the requested bbox")

    # Reprojecting a 10m coastline to Mercator reliably produces rings that
    # touch at a point. Cleaning here, once, keeps every chunk from paying for
    # the overlay retries.
    mask = snap_clean(to_mercator(merged))

    log(f"land mask: {len(list(geometry_parts(mask)))} polygons")
    return mask


def geometry_parts(geometry: Any) -> Iterator[Any]:
    """
    Split a MultiPolygon into its polygons.

    One planet-wide MultiPolygon is a single tippecanoe feature: it is kept or
    dropped as a unit, and it cannot be coalesced. Individual polygons let
    tippecanoe drop only what is genuinely too small for a zoom.
    """

    if geometry is None:
        return
    if geometry.geom_type == "MultiPolygon":
        yield from geometry.geoms
    else:
        yield geometry


# ============================================================================
# BATHYMETRY
# ============================================================================

def load_bathy_contours(directory: Path, bbox: Bbox) -> dict[int, Any]:
    contours: dict[int, Any] = {}
    window = box(*bbox)

    for depth in BATHY_LEVELS:
        letter = BATHY_LETTERS[depth]
        matches = sorted(directory.rglob(f"ne_10m_bathymetry_{letter}_{depth}.shp"))
        if not matches:
            fail(f"cannot find Natural Earth bathymetry for {depth} m")

        frame = gpd.read_file(matches[0], bbox=None if is_global(bbox) else bbox)
        if frame.empty:
            contours[depth] = None
            continue

        frame = frame.set_crs(WGS84) if frame.crs is None else frame.to_crs(WGS84)
        parts = []
        for geometry in frame.geometry:
            geometry = polygon_only(geometry) if is_global(bbox) else clip(geometry, window)
            if geometry is not None:
                parts.append(geometry)

        contours[depth] = dissolve(parts)

    return contours


def build_bathymetry(directory: Path, bbox: Bbox, output: Path,
                     min_zoom: int, max_zoom: int, simplify_pixels: float,
                     min_zone_pixels: float, force: bool) -> Path:
    """
    Depth as nested polygons, shallowest first.

    Class N is the *whole* contour(N) polygon, not the ring between contour(N)
    and contour(N+1), so the classes stack instead of tiling. This is what
    Mapbox does, and it is better on all three counts:

    - No holes are possible. A ring that loses a piece to the size filter
      leaves bare background; a nested polygon that loses one just shows the
      shallower colour already painted underneath.
    - Half the vertices. A ring carries both its own contour and the next one
      inwards, so every contour is stored twice; nested stores each once.
    - Simplification is unconditionally safe. Rings cut from a shared contour
      have to be simplified before they are cut or they come apart; stacked
      polygons never share an edge to begin with.

    The cost is that the style must paint the classes in order, 0 to 11, deep
    over shallow. `scripts/bathymap-style.json` has that.
    """

    if cached(output, force, BATHY_OUTPUT_LAYER):
        return output

    stage(BATHY_OUTPUT_LAYER)

    contours = {
        depth: None if geometry is None else to_mercator(geometry)
        for depth, geometry in load_bathy_contours(directory, bbox).items()
    }

    if not any(geometry is not None for geometry in contours.values()):
        fail("no bathymetry geometry generated for this bbox")

    written = 0

    with FeatureWriter(output) as writer:
        for zoom in range(min_zoom, max_zoom + 1):
            # Reduce the contours *before* differencing them. Reducing the
            # finished bands instead shatters every ring that is thinner than
            # a pixel into thousands of slivers - at z0 that alone was 55k
            # features and an unbuildable tile - and leaves gaps between bands
            # that were cut from the same line.
            shallowest = BATHY_LEVELS[0]

            for spec in BATHY_CLASSES:
                geometry = contours.get(spec["min"])
                if geometry is None:
                    continue

                # The shallowest contour is the sea itself: dropping a piece
                # of it leaves real background showing, with nothing beneath
                # to fall back to. Every deeper one can be filtered freely.
                geometry = reduce_for_zoom(
                    geometry, zoom,
                    simplify_pixels=simplify_pixels,
                    min_pixels=(
                        MIN_PIXELS if spec["min"] == shallowest else min_zone_pixels
                    ),
                )
                if geometry is None:
                    continue

                for part in geometry_parts(to_wgs84(geometry)):
                    writer.write(part, {BATHY_PROPERTY: spec["min"]},
                                 minzoom=zoom, maxzoom=zoom)
                    written += 1

        count = writer.count
        writer.close()

    if count == 0:
        fail("no bathymetry geometry survived at any zoom")

    log(f"depth: {count} polygons across z{min_zoom}-{max_zoom}")
    return output


# ============================================================================
# LANDCOVER SOURCE
# ============================================================================

def build_landcover_region(versatiles: str, cache: Path, bbox: Bbox,
                           min_zoom: int, max_zoom: int, refresh: bool) -> Path:
    """
    Pull the bbox out of the remote VersaTiles container, once.

    Written to a temporary name and renamed, because a run killed halfway used
    to leave a truncated container that every later run happily treated as
    cached. The temporary name keeps the `.versatiles` extension - versatiles
    picks the container format from it and rejects `...versatiles.partial`
    with "file extension 'partial' unknown".
    """

    directory = cache / "landcover" / region_key(bbox, min_zoom, max_zoom)
    directory.mkdir(parents=True, exist_ok=True)
    output = directory / "region.versatiles"

    if output.exists() and output.stat().st_size > 0 and not refresh:
        log(f"cached regional landcover: {output} "
            f"({output.stat().st_size / 1e6:.1f} MB)")
        return output

    stage("landcover_source")

    partial = output.with_name("region.partial.versatiles")
    partial.unlink(missing_ok=True)

    run([
        versatiles, "convert",
        "--bbox", bbox_string(bbox),
        "--bbox-border", str(LANDCOVER_BBOX_BORDER),
        "--min-zoom", str(min_zoom),
        "--max-zoom", str(max_zoom),
        LANDCOVER_URL,
        str(partial),
    ])

    if not partial.exists() or partial.stat().st_size == 0:
        fail("versatiles produced no regional landcover container")

    partial.replace(output)
    return output


def export_landcover_tiles(versatiles: str, source: Path, output_dir: Path) -> Path:
    """
    Explode the container into plain, uncompressed MVT tiles.

    `-c uncompressed` is the whole point. VersaTiles keeps the source
    compression by default, and the upstream container switched to brotli:
    `.pbf.br` is neither recognised as a tile nor readable by GDAL, so the
    entire landcover step failed with "no recognizable MVT/PBF tiles".

    Cached on the container's size and mtime rather than on `--force`, since
    it is a pure function of the download.
    """

    marker = output_dir / ".exported"
    signature = f"{source.stat().st_size}:{int(source.stat().st_mtime)}"

    if marker.exists() and marker.read_text(encoding="utf-8").strip() == signature:
        log(f"cached MVT tiles: {output_dir}")
        return output_dir

    stage("landcover_tiles")

    if output_dir.exists():
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    run([versatiles, "convert", "-c", "uncompressed", str(source), str(output_dir)])

    if not any(output_dir.rglob("*.pbf")):
        fail(f"no .pbf tiles were written to {output_dir}")

    marker.write_text(signature, encoding="utf-8")
    return output_dir


def available_source_zooms(tile_dir: Path) -> set[int]:
    zooms = set()
    for entry in tile_dir.iterdir():
        if entry.is_dir() and entry.name.isdigit() and any(entry.rglob("*.pbf")):
            zooms.add(int(entry.name))
    return zooms


def pick_source_zoom(zoom: int, available: set[int]) -> int | None:
    """Prefer the requested zoom, then the closest coarser one, then finer."""

    if zoom in available:
        return zoom
    lower = [z for z in available if z < zoom]
    if lower:
        return max(lower)
    higher = [z for z in available if z > zoom]
    if higher:
        return min(higher)
    return None


def read_source_tile(tile_dir: Path, zoom: int, x: int, y: int) -> gpd.GeoDataFrame | None:
    """
    Read one MVT tile, buffer included.

    `CLIP=NO` keeps the geometry that spills past the tile edge, which is what
    lets a dissolve stitch two neighbouring tiles into one polygon instead of
    leaving a hairline seam. It has to be passed as a pyogrio keyword - the
    `open_options=` form is silently ignored and warns about an unknown
    option named OPEN_OPTIONS.
    """

    path = tile_dir / str(zoom) / str(x) / f"{y}.pbf"
    if not path.is_file():
        return None

    try:
        frame = gpd.read_file(
            f"MVT:{path}",
            layer=LANDCOVER_LAYER,
            engine="pyogrio",
            CLIP="NO",
        )
    except Exception:
        return None

    if frame.empty or LANDCOVER_CLASS_FIELD not in frame.columns:
        return None

    frame = frame[[LANDCOVER_CLASS_FIELD, "geometry"]].copy()
    frame["class"] = frame[LANDCOVER_CLASS_FIELD].map(
        lambda value: LANDCOVER_CLASS_MAP.get(str(value).strip().lower())
        if value is not None else None
    )
    frame = frame[frame["class"].notna()]
    if frame.empty:
        return None

    if frame.crs is None:
        frame = frame.set_crs(WEB_MERCATOR)
    elif frame.crs.to_epsg() != 3857:
        frame = frame.to_crs(WEB_MERCATOR)

    return frame[["class", "geometry"]]


# ============================================================================
# LANDCOVER RECONSTRUCTION
# ============================================================================

def resolve_priority(classes: dict[str, Any]) -> dict[str, Any]:
    """
    Cut overlaps out, highest priority first.

    Two classes covering the same ground render as z-fighting fill, and the
    cumulative fill below only works if "what this zoom already covers" is a
    single unambiguous area.
    """

    resolved: dict[str, Any] = {}
    claimed: Any = None

    for name in LANDCOVER_PRIORITY:
        geometry = classes.get(name)
        if geometry is None:
            continue

        if claimed is not None:
            geometry = polygon_only(geometry.difference(claimed))
            if geometry is None:
                continue

        resolved[name] = geometry
        claimed = geometry if claimed is None else polygon_only(
            shapely.union_all([claimed, geometry])
        )

    # Anything outside the priority list would otherwise vanish silently.
    for name, geometry in classes.items():
        if name not in LANDCOVER_PRIORITY and geometry is not None:
            resolved.setdefault(name, geometry)

    return resolved


def read_chunk_result(path: Path) -> dict[str, Any]:
    """
    Load one chunk's dissolved classes, or nothing if it has not been built.

    An empty chunk is a zero-byte file rather than a missing one, so that a
    resumed run does not redo the ocean.
    """

    if not path.is_file() or path.stat().st_size == 0:
        return {}

    frame = gpd.read_file(path)
    if frame.empty:
        return {}

    return {
        str(row["class"]): row.geometry
        for _, row in frame.iterrows()
        if row.geometry is not None and not row.geometry.is_empty
    }


def write_chunk_result(path: Path, classes: dict[str, Any]) -> None:
    """
    Write one chunk's classes atomically.

    The temporary name has to keep the `.fgb` extension. GDAL picks the
    FlatGeobuf single-file layout from the extension and falls back to a
    *directory* of layers without it - so a `<name>.fgb.partial` target
    produced a directory, `is_file()` on the renamed result was false, every
    chunk read back as empty, and the whole cumulative fill silently did
    nothing.
    """

    path.parent.mkdir(parents=True, exist_ok=True)

    if not classes:
        path.write_bytes(b"")
        return

    frame = gpd.GeoDataFrame(
        {"class": list(classes.keys())},
        geometry=list(classes.values()),
        crs=WEB_MERCATOR,
    )

    partial = path.with_name(f"{path.stem}.partial{path.suffix}")
    if partial.is_dir():
        shutil.rmtree(partial)
    else:
        partial.unlink(missing_ok=True)

    frame.to_file(partial, driver="FlatGeobuf")

    # A previous version of this script left directories here.
    if path.is_dir():
        shutil.rmtree(path)
    partial.replace(path)


def build_chunk(tile_dir: Path, source_zoom: int, zoom: int, chunk_zoom: int,
                cx: int, cy: int, window: Any, extras: dict[str, Any],
                base: dict[str, Any], smooth_pixels: float,
                min_zone_pixels: float, simplify_pixels: float) -> dict[str, Any]:
    """
    One chunk of one zoom: dissolve, generalise, then inherit what is missing.

    `window` is the chunk's own bounds intersected with the requested bbox and
    with the coastline, in Web Mercator. Everything is cut on it, so
    neighbouring chunks meet on exactly coincident edges and nothing reaches
    past the shore.

    Order matters. Generalising before the priority pass is what keeps the
    result overlap-free: the buffers deliberately grow each class into its
    neighbours, and `resolve_priority` is what cuts them apart again.
    """

    frames = []
    for x, y in child_tiles(source_zoom, chunk_zoom, cx, cy):
        frame = read_source_tile(tile_dir, source_zoom, x, y)
        if frame is not None:
            frames.append(frame)

    detail: dict[str, Any] = {}

    if frames:
        combined = pd.concat(frames, ignore_index=True)
        for name, group in combined.groupby("class", sort=False):
            merged = dissolve(group.geometry.values)
            if merged is not None:
                detail[str(name)] = merged

    # Natural Earth glaciers and playas are vector at 10m for every zoom, so
    # they go in as detail rather than being inherited from a coarser zoom.
    for name, geometry in extras.items():
        piece = clip(geometry, window)
        if piece is None:
            continue
        detail[name] = piece if name not in detail else dissolve([detail[name], piece])

    generalized: dict[str, Any] = {}
    for name, geometry in detail.items():
        zones = generalize(geometry, zoom, smooth_pixels, min_zone_pixels)
        if zones is None:
            continue
        zones = clip(zones, window)
        if zones is not None:
            generalized[name] = zones

    detail = resolve_priority(generalized)
    detail = simplify_coverage(detail, zoom, simplify_pixels)

    # The priority pass cuts holes into the lower classes, which can leave
    # slivers behind. They are not worth a second smoothing pass, but they are
    # worth dropping - whatever they vacate is filled from the zoom below.
    grid = zoom_grid(zoom)
    kept_detail = {
        name: kept
        for name, kept in (
            (n, drop_small(g, min_zone_pixels * grid * grid)) for n, g in detail.items()
        )
        if kept is not None
    }

    if not base:
        return rescue_islands(kept_detail, window, detail, MIN_PIXELS * grid * grid)

    detail, full_detail = kept_detail, detail

    covered = dissolve(detail.values()) if detail else None
    result = dict(detail)

    for name, geometry in base.items():
        inherited = clip(geometry, window)
        if inherited is None:
            continue
        if covered is not None:
            inherited = polygon_only(inherited.difference(covered))
            if inherited is None:
                continue
        result[name] = (
            inherited if name not in result else dissolve([result[name], inherited])
        )

    return rescue_islands(result, window, full_detail, MIN_PIXELS * grid * grid)


# Set once per worker process by `_init_worker`. The coastline mask and the
# Natural Earth extras are the only large inputs a chunk needs that do not come
# off disk already, and they are identical for every chunk - so they travel as
# two files the workers read once, rather than as pickled geometry attached to
# every one of the hundreds of tasks.
_WORKER: dict[str, Any] = {}


def _init_worker(mask_path: str, extras_path: str) -> None:
    _WORKER["mask"] = read_chunk_result(Path(mask_path)).get("mask")
    _WORKER["extras"] = read_chunk_result(Path(extras_path))


def _chunk_task(job: dict[str, Any]) -> tuple[int, int]:
    """
    Build one chunk in a worker and write it.

    The result goes to disk rather than back through the queue: the parent has
    to be able to read these files anyway to resume an interrupted run, and a
    dissolved chunk is far too big to want to pickle.
    """

    window = chunk_window(
        job["chunk_zoom"], job["cx"], job["cy"],
        shapely.from_wkb(job["request"]), _WORKER["mask"],
    )

    if window is None:
        classes: dict[str, Any] = {}
    else:
        classes = build_chunk(
            Path(job["tile_dir"]), job["source_zoom"], job["zoom"],
            job["chunk_zoom"], job["cx"], job["cy"], window,
            _WORKER["extras"],
            read_chunk_result(Path(job["base_path"])) if job["base_path"] else {},
            job["smooth"], job["zone_pixels"], job["simplify"],
        )

    write_chunk_result(Path(job["chunk_path"]), classes)
    return job["cx"], job["cy"]


def resolve_jobs(requested: int | None) -> int:
    if requested is not None:
        return max(1, requested)
    return max(1, min(16, (os.cpu_count() or 2) - 2))


def build_landcover(versatiles: str, cache: Path, work: Path, bbox: Bbox,
                    min_zoom: int, max_zoom: int, chunk_zoom: int,
                    directories: dict[str, Path], land_mask: Callable[[], Any],
                    output: Path, smooth_pixels: float, min_zone_pixels: float,
                    simplify_pixels: float, jobs: int,
                    force: bool, refresh_source: bool) -> Path:

    if cached(output, force, "landcover"):
        return output

    # Only now, because unioning the world's coastline is a few seconds that a
    # fully cached run has no reason to spend.
    mask = land_mask()

    source = build_landcover_region(
        versatiles, cache, bbox, min_zoom, max_zoom, refresh_source
    )
    tile_dir = export_landcover_tiles(versatiles, source, source.parent / "mvt")

    available = available_source_zooms(tile_dir)
    if not available:
        fail(f"no MVT zoom directories under {tile_dir}")
    log(f"source zooms available: {sorted(available)}")

    stage("landcover")

    extras = load_landcover_extras(directories, bbox)

    chunks = tiles_covering(bbox, chunk_zoom)
    log(f"chunk grid: z{chunk_zoom}, {len(chunks)} chunks")

    request_window_4326 = box(*bbox)

    # Keyed by the grid, because (cx, cy) means a different square at every
    # chunk zoom. Sharing one directory let a re-run with a different
    # --chunk-zoom read its predecessor's chunks as if they were its own.
    chunk_root = work / f"chunks-z{chunk_zoom}"

    # The two shared inputs, staged where workers can pick them up.
    mask_path = work / "land-mask.fgb"
    extras_path = work / "landcover-extras.fgb"
    write_chunk_result(mask_path, {"mask": mask})
    write_chunk_result(extras_path, extras)

    workers = min(jobs, len(chunks))
    pool = None
    if workers > 1:
        # spawn, because fork with a loaded GEOS is not safe. That means the
        # module is re-imported per worker, which is why nothing expensive
        # happens at import time.
        pool = concurrent.futures.ProcessPoolExecutor(
            max_workers=workers,
            mp_context=multiprocessing.get_context("spawn"),
            initializer=_init_worker,
            initargs=(str(mask_path), str(extras_path)),
        )
        log(f"building chunks on {workers} processes")

    request_wkb = shapely.to_wkb(request_window_4326)

    try:
      with FeatureWriter(output) as writer:
        for zoom in range(min_zoom, max_zoom + 1):
            source_zoom = pick_source_zoom(zoom, available)
            if source_zoom is None:
                fail("the regional landcover container has no usable zoom")
            if source_zoom != zoom:
                log(f"z{zoom}: no source at this zoom, using z{source_zoom}")

            written_before = writer.count

            # Generalising is only safe because the zoom below fills what it
            # removes - both the zones dropped for being small and the coast
            # the opening erodes. The very first zoom has nothing below it, so
            # it is built raw; smoothing it cost 10% of the world's land.
            smooth = smooth_pixels if zoom > min_zoom else 0.0
            zone_pixels = min_zone_pixels if zoom > min_zoom else MIN_PIXELS

            # One zoom is a barrier: every chunk of it reads the same chunk of
            # the zoom below. Within a zoom the chunks are independent, and on
            # a global build that is where all the time goes.
            todo = []
            for cx, cy in chunks:
                chunk_path = chunk_root / f"z{zoom}" / f"{cx}-{cy}.fgb"
                if chunk_path.exists() and not force:
                    continue
                todo.append({
                    "tile_dir": str(tile_dir),
                    "source_zoom": source_zoom,
                    "zoom": zoom,
                    "chunk_zoom": chunk_zoom,
                    "cx": cx, "cy": cy,
                    "request": request_wkb,
                    "chunk_path": str(chunk_path),
                    "base_path": (
                        str(chunk_root / f"z{zoom - 1}" / f"{cx}-{cy}.fgb")
                        if zoom > min_zoom else ""
                    ),
                    "smooth": smooth,
                    "zone_pixels": zone_pixels,
                    "simplify": simplify_pixels,
                })

            done = 0
            if pool is not None and todo:
                for _ in pool.map(_chunk_task, todo):
                    done += 1
                    progress(f"landcover-z{zoom}", done, len(todo))
            else:
                _WORKER["mask"] = mask
                _WORKER["extras"] = extras
                for job in todo:
                    _chunk_task(job)
                    done += 1
                    progress(f"landcover-z{zoom}", done, len(todo))

            for cx, cy in chunks:
                classes = read_chunk_result(
                    chunk_root / f"z{zoom}" / f"{cx}-{cy}.fgb"
                )
                for name, geometry in classes.items():
                    reduced = reduce_for_zoom(geometry, zoom)
                    if reduced is None:
                        continue
                    for part in geometry_parts(to_wgs84(reduced)):
                        writer.write(part, {"class": name}, minzoom=zoom, maxzoom=zoom)

            log(f"z{zoom}: {writer.count - written_before} polygons")

        count = writer.count
        writer.close()
    finally:
        if pool is not None:
            pool.shutdown()

    if count == 0:
        fail("no landcover geometry generated")

    log(f"landcover features: {count}")
    return output


def chunk_window(chunk_zoom: int, cx: int, cy: int, request_4326: Any,
                 land_mask: Any) -> Any:
    """
    The chunk's bounds, cut to the requested bbox and to the coastline.

    Folding the coastline into the window rather than clipping each class
    against it separately means the intersection is done once per chunk
    instead of once per class per chunk, and a chunk that is all ocean is
    recognised before any source tile is read.
    """

    chunk_3857 = box(*tile_bounds_3857(chunk_zoom, cx, cy))
    request_3857 = (
        gpd.GeoSeries([request_4326], crs=WGS84).to_crs(WEB_MERCATOR).iloc[0]
    )
    window = chunk_3857.intersection(request_3857)
    if window.is_empty:
        return None

    return polygon_only(safe_intersection(window, land_mask))


def to_wgs84(geometry: Any) -> Any:
    return gpd.GeoSeries([geometry], crs=WEB_MERCATOR).to_crs(WGS84).iloc[0]


def load_landcover_extras(directories: dict[str, Path], bbox: Bbox) -> dict[str, Any]:
    """
    Classes the VersaTiles container cannot supply above z3.

    Its `water_polygons` layer stops at z3, so glaciers would disappear on
    exactly the zooms where Greenland and the Alps matter. Natural Earth has
    them as 10m vectors at every zoom, which is both better data and simpler
    than inheriting a z3 blob upward.
    """

    extras: dict[str, Any] = {}

    glacier_parts = []
    for name, hint in (
        ("glaciated", "ne_10m_glaciated_areas"),
        ("ice_shelves", "ne_10m_antarctic_ice_shelves_polys"),
    ):
        geometry = load_shapefile(directories[name], hint, bbox)
        if geometry is not None:
            glacier_parts.append(geometry)

    glaciers = dissolve(glacier_parts)
    if glaciers is not None:
        extras["glacier"] = to_mercator(glaciers)

    playas = load_shapefile(directories["playas"], "ne_10m_playas", bbox)
    if playas is not None:
        extras["sand"] = to_mercator(playas)

    log(f"extra classes from Natural Earth: {sorted(extras)}")
    return extras


def to_mercator(geometry: Any) -> Any:
    return gpd.GeoSeries([geometry], crs=WGS84).to_crs(WEB_MERCATOR).iloc[0]


# ============================================================================
# FINAL MBTILES
# ============================================================================

def build_mbtiles(tippecanoe: str, layers: list[tuple[str, Path]], output: Path,
                  min_zoom: int, max_zoom: int, detail: int, force: bool) -> None:

    if cached(output, force, "MBTiles", [path for _, path in layers]):
        return

    stage("mbtiles")

    output.parent.mkdir(parents=True, exist_ok=True)
    # Keep the output's own extension. Tippecanoe picks the container from the suffix, so a
    # hardcoded `.partial.mbtiles` made it write an MBTiles that was then renamed to
    # `.pmtiles` - a SQLite file with a PMTiles name, which nothing can read.
    partial = output.with_name(f"{output.stem}.partial{output.suffix}")
    partial.unlink(missing_ok=True)

    command = [
        tippecanoe,
        "--force",
        "--output", str(partial),
        "--minimum-zoom", str(min_zoom),
        "--maximum-zoom", str(max_zoom),
        "--full-detail", str(detail),
        "--buffer", str(TIPPECANOE_BUFFER),

        # Keep polygons that share an edge sharing it after simplification;
        # without this, adjacent landcover classes develop visible cracks.
        "--detect-shared-borders",
        "--no-simplification-of-shared-nodes",

        # Low-zoom landcover is mostly small polygons. Reducing them to dots
        # or dropping them is exactly the confetti this file exists to avoid.
        "--no-tiny-polygon-reduction",
        "--coalesce-densest-as-needed",

        "--name", "Alpimaps Bathymap",
        "--description", "Global landcover and bathymetry for low zooms",
        "--attribution",
        "© Natural Earth; © ESA WorldCover project 2021 / VersaTiles",
    ]

    for name, path in layers:
        command += ["-L", f"{name}:{path}"]

    run(command)

    if not partial.exists() or partial.stat().st_size == 0:
        fail("tippecanoe produced no MBTiles")

    partial.replace(output)
    log(f"final MBTiles: {output} ({output.stat().st_size / 1e6:.1f} MB)")


def write_metadata(output: Path, bbox: Bbox, min_zoom: int, max_zoom: int,
                   chunk_zoom: int) -> Path:

    path = output.with_suffix(".json")
    path.write_text(
        json.dumps(
            {
                "pipeline_version": PIPELINE_VERSION,
                "format": "MVT",
                "file": output.name,
                "bounds": list(bbox),
                "zoom": {"min": min_zoom, "max": max_zoom},
                "chunk_zoom": chunk_zoom,
                "layers": {
                    LANDCOVER_OUTPUT_LAYER: {
                        "property": "class",
                        "classes": LANDCOVER_PRIORITY,
                        "cumulative": True,
                        "clipped_to_coastline": True,
                    },
                    BATHY_OUTPUT_LAYER: {
                        "property": BATHY_PROPERTY,
                        "values": BATHY_LEVELS,
                        "unit": "metre",
                        "nested": True,
                        "meaning": "the isobath; every point inside the "
                                   "polygon is at least min_depth deep",
                        "draw_order": "ascending min_depth; each polygon is a "
                                      "whole contour, so deeper must paint "
                                      "over shallower",
                    },
                },
                "sources": {
                    LANDCOVER_OUTPUT_LAYER:
                        "VersaTiles landcover-vectors / ESA WorldCover 2021",
                    "coastline": "Natural Earth 10m land + minor islands",
                    "glacier": "Natural Earth 10m glaciated areas + ice shelves",
                    BATHY_OUTPUT_LAYER: "Natural Earth 10m bathymetry",
                },
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    return path


# ============================================================================
# MAIN
# ============================================================================

def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build one vector MBTiles with global landcover and bathymetry."
    )

    parser.add_argument("--bbox", nargs=4, type=float,
                        metavar=("WEST", "SOUTH", "EAST", "NORTH"))
    parser.add_argument("--global", action="store_true", dest="global_world",
                        help="build the whole world instead of a bbox")
    parser.add_argument("--output", required=True,
                        help="output MBTiles path, e.g. output/mediterranean.mbtiles")

    parser.add_argument("--cache", default=".cache/bathymap",
                        help="persistent cache directory (default: .cache/bathymap)")
    parser.add_argument("--min-zoom", type=int, default=DEFAULT_MIN_ZOOM)
    parser.add_argument("--max-zoom", type=int, default=DEFAULT_MAX_ZOOM)
    parser.add_argument("--chunk-zoom", type=int, default=None,
                        help="zoom of the processing grid; by default the "
                             "coarsest grid the region fits in, which is a "
                             "single seamless chunk for anything regional")

    generalisation = parser.add_argument_group(
        "generalisation",
        "All of these are in tile units at the zoom being built, so they mean "
        "the same thing on screen at every zoom. Bigger is smoother.",
    )
    generalisation.add_argument(
        "--landcover-smooth-pixels", type=float, default=SMOOTH_PIXELS,
        help="landcover smoothing radius; 0 disables the smoothing pass "
             f"(default: {SMOOTH_PIXELS:g})")
    generalisation.add_argument(
        "--landcover-min-zone-pixels", type=float, default=MIN_ZONE_PIXELS,
        help="smallest landcover zone to keep, squared; what is dropped is "
             f"filled from the zoom below (default: {MIN_ZONE_PIXELS:g})")
    generalisation.add_argument(
        "--landcover-simplify-pixels", type=float, default=LANDCOVER_SIMPLIFY_PIXELS,
        help="Douglas-Peucker tolerance for landcover, applied to every class "
             "at once so shared borders survive; 0 disables it "
             f"(default: {LANDCOVER_SIMPLIFY_PIXELS:g})")
    generalisation.add_argument(
        "--bathymetry-simplify-pixels", type=float, default=BATHY_SIMPLIFY_PIXELS,
        help="Douglas-Peucker tolerance for the depth contours; 0 leaves them "
             f"at full 10m detail (default: {BATHY_SIMPLIFY_PIXELS:g})")
    generalisation.add_argument(
        "--bathymetry-min-zone-pixels", type=float, default=BATHY_MIN_ZONE_PIXELS,
        help="smallest depth-band piece to keep, squared "
             f"(default: {BATHY_MIN_ZONE_PIXELS:g})")

    parser.add_argument("--detail", type=int, default=DEFAULT_DETAIL,
                        help="tile coordinate resolution as a power of two "
                             f"(default: {DEFAULT_DETAIL}; 12 is tippecanoe's "
                             "own default, 10 is a quarter of the precision "
                             "and about 25%% smaller)")

    parser.add_argument("--jobs", "-j", type=int, default=None,
                        help="worker processes for the landcover chunks "
                             "(default: cores - 2, capped at 16; 1 to disable)")

    parser.add_argument("--versatiles", help="path to the versatiles executable")
    parser.add_argument("--tippecanoe", help="path to the tippecanoe executable")

    parser.add_argument("--force", action="store_true",
                        help="rebuild derived products; downloads stay cached")
    parser.add_argument("--refresh-source", action="store_true",
                        help="re-fetch the regional landcover extract as well")
    parser.add_argument("--dry-run", action="store_true",
                        help="print the plan and the resolved paths, then exit")

    args = parser.parse_args(argv)

    if args.bbox is None and not args.global_world:
        parser.error("provide --bbox or --global")
    if args.bbox is not None and args.global_world:
        parser.error("provide only one of --bbox or --global")
    if args.min_zoom < 0 or args.max_zoom < args.min_zoom:
        parser.error("--max-zoom must be >= --min-zoom >= 0")
    if args.chunk_zoom is not None and args.chunk_zoom < 0:
        parser.error("--chunk-zoom must be >= 0")
    if args.jobs is not None and args.jobs < 1:
        parser.error("--jobs must be >= 1")
    if not 8 <= args.detail <= 14:
        parser.error("--detail must be between 8 and 14")
    for name in ("landcover_smooth_pixels", "landcover_min_zone_pixels",
                 "landcover_simplify_pixels",
                 "bathymetry_simplify_pixels", "bathymetry_min_zone_pixels"):
        if getattr(args, name) < 0:
            parser.error(f"--{name.replace('_', '-')} must be >= 0")

    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)

    bbox: Bbox = (
        (-180.0, -MERCATOR_MAX_LAT, 180.0, MERCATOR_MAX_LAT)
        if args.global_world
        else tuple(args.bbox)  # type: ignore[assignment]
    )
    validate_bbox(bbox)

    versatiles = resolve_tool("versatiles", args.versatiles)
    tippecanoe = resolve_tool("tippecanoe", args.tippecanoe)

    cache = Path(args.cache)
    key = region_key(bbox, args.min_zoom, args.max_zoom)
    work = cache / "work" / key / f"v{PIPELINE_VERSION}"
    output = Path(args.output)

    jobs = resolve_jobs(args.jobs)
    chunk_zoom = choose_chunk_zoom(bbox, args.max_zoom, jobs, args.chunk_zoom)

    log("=" * 60)
    log(f"Alpimaps Bathymap - pipeline v{PIPELINE_VERSION}")
    log(f"bbox:       {bbox_string(bbox)}{' (global)' if is_global(bbox) else ''}")
    chosen = "chosen" if args.chunk_zoom is None else "requested"
    log(f"zoom:       {args.min_zoom}-{args.max_zoom}, "
        f"chunk grid z{chunk_zoom} ({chosen})")
    log(f"landcover:  smooth {args.landcover_smooth_pixels:g} px, "
        f"simplify {args.landcover_simplify_pixels:g} px, "
        f"min zone {args.landcover_min_zone_pixels:g} px2")
    log(f"bathymetry: simplify {args.bathymetry_simplify_pixels:g} px, "
        f"min zone {args.bathymetry_min_zone_pixels:g} px2")
    log(f"tiles:      detail {args.detail}, buffer {TIPPECANOE_BUFFER}")
    log(f"jobs:       {jobs}")
    log(f"region:     {key}")
    log(f"cache:      {cache}")
    log(f"work:       {work}")
    log(f"versatiles: {versatiles}")
    log(f"tippecanoe: {tippecanoe}")
    log(f"output:     {output}")
    log("=" * 60)

    if args.dry_run:
        log("dry run: nothing was downloaded or built")
        return 0

    if args.force:
        log("force: derived products will be rebuilt; downloads stay cached")

    work.mkdir(parents=True, exist_ok=True)

    stage("natural_earth")
    directories = download_natural_earth(cache)

    depth = build_bathymetry(
        directories["bathymetry"], bbox, work / f"{BATHY_OUTPUT_LAYER}.geojsonl",
        args.min_zoom, args.max_zoom,
        args.bathymetry_simplify_pixels, args.bathymetry_min_zone_pixels,
        args.force,
    )

    landcover = build_landcover(
        versatiles, cache, work, bbox, args.min_zoom, args.max_zoom,
        chunk_zoom, directories,
        lambda: load_land_mask(directories, bbox),
        work / f"{LANDCOVER_OUTPUT_LAYER}.geojsonl",
        args.landcover_smooth_pixels, args.landcover_min_zone_pixels,
        args.landcover_simplify_pixels, jobs, args.force, args.refresh_source,
    )

    build_mbtiles(
        tippecanoe,
        [(LANDCOVER_OUTPUT_LAYER, landcover), (BATHY_OUTPUT_LAYER, depth)],
        output, args.min_zoom, args.max_zoom, args.detail, args.force,
    )

    metadata = write_metadata(output, bbox, args.min_zoom, args.max_zoom, chunk_zoom)

    stage("done")
    log(f"MBTiles:  {output}")
    log(f"metadata: {metadata}")
    log(f"work:     {work}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BathymapError as error:
        print(f"[bathymap] ERROR {error}", file=sys.stderr, flush=True)
        raise SystemExit(1)
    except KeyboardInterrupt:
        print("[bathymap] interrupted", file=sys.stderr, flush=True)
        raise SystemExit(130)
