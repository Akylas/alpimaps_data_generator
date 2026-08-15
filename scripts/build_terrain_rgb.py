#!/usr/bin/env python
"""
Build terrain RGB mbtiles from any number of elevation sources, mapterhorn style:
the max zoom is warped once per macrotile and every lower zoom is a 2x2 average of
the tiles already produced, instead of one full pass over the dem per zoom.

Sources are listed lowest priority first. Each one is warped into the macrotile
grid and composited with a feathered mask, so a better source overrides a coarser
one without leaving a step along the edge of its coverage.
"""

from __future__ import division

import argparse
import glob as globlib
import io
import json
import math
import os
import re
import sqlite3
import subprocess
import sys
import urllib.request
from multiprocessing import Pool

import mercantile
import numpy as np
import rasterio
from PIL import Image
from rasterio.enums import Resampling
from rasterio.warp import reproject
from rasterio import transform as riotransform
from shapely import geometry

from rio_rgbify.encoders import data_to_rgb
from rio_rgbify.mbtiler import parse_poly

CATALOG_URL = "https://raw.githubusercontent.com/mapterhorn/mapterhorn/main/source-catalog"

# base value / interval of the supported encodings. terrarium is the same
# base 256 encoding as mapbox, with base -32768 and a 1/256 interval:
#   -32768 + (R*65536 + G*256 + B)/256 == (R*256 + G + B/256) - 32768
ENCODINGS = {
    "mapbox": (-10000.0, 0.1),
    "terrarium": (-32768.0, 1.0 / 256.0),
}

# filenames of the 1x1 degree sources carry their south west corner, which is
# enough to skip the downloads outside of the area being built
DEGREE_TILE_RE = re.compile(r"(?P<ns>[NS])(?P<lat>\d{2})_00_(?P<ew>[EW])(?P<lon>\d{3})")

WORKER = {}


def log(msg):
    print(msg, file=sys.stderr, flush=True)


# --------------------------------------------------------------------------
# sources
# --------------------------------------------------------------------------


def _degree_tile_bounds(url):
    m = DEGREE_TILE_RE.search(os.path.basename(url))
    if m is None:
        return None
    lat = int(m.group("lat")) * (1 if m.group("ns") == "N" else -1)
    lon = int(m.group("lon")) * (1 if m.group("ew") == "E" else -1)
    return (lon, lat, lon + 1, lat + 1)


def _intersects(a, b):
    return a[0] < b[2] and a[2] > b[0] and a[1] < b[3] and a[3] > b[1]


# some providers, data.geopf.fr among them, answer 403 to the default
# `Python-urllib/x.y` user agent. mapterhorn shells out to wget and never sees it
USER_AGENT = "alpimaps_data_generator/1.0"


def _human(n):
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return "%.1f%s" % (n, unit) if unit != "B" else "%dB" % n
        n /= 1024.0


def _download(url, dest, retries=4):
    if os.path.exists(dest) and os.path.getsize(dest) > 0:
        return dest
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    tmp = dest + ".part"

    for attempt in range(retries):
        offset = os.path.getsize(tmp) if os.path.exists(tmp) else 0
        req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        mode = "wb"
        if offset:
            # these deliveries are several GB per part, resume rather than restart
            req.add_header("Range", "bytes=%d-" % offset)
            mode = "ab"
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                if mode == "ab" and r.status != 206:
                    mode, offset = "wb", 0  # server ignored the range
                total = r.headers.get("Content-Length")
                log(
                    "  download %s%s%s"
                    % (
                        os.path.basename(url),
                        " (resume at %s)" % _human(offset) if offset else "",
                        " of %s" % _human(int(total) + offset) if total else "",
                    )
                )
                with open(tmp, mode) as f:
                    while True:
                        chunk = r.read(1 << 20)
                        if not chunk:
                            break
                        f.write(chunk)
            os.rename(tmp, dest)
            return dest
        except Exception as exc:
            if attempt == retries - 1:
                raise
            log("  %s: %s, retrying" % (os.path.basename(url), exc))
    return dest


def _build_vrt(files, vrt_path, extra=()):
    if not files:
        raise SystemExit("no file to build %s from" % vrt_path)
    listfile = vrt_path + ".txt"
    with open(listfile, "w") as f:
        f.write("\n".join(files))
    cmd = ["gdalbuildvrt", "-q"] + list(extra) + ["-input_file_list", listfile, vrt_path]
    subprocess.run(cmd, check=True)
    os.remove(listfile)
    return vrt_path


def prepare_valhalla(spec, bbox, workdir):
    """.hgt tiles already on disk for valhalla, downloading the missing ones"""
    path = spec["path"]
    if spec.get("download", True):
        bounds = "%s,%s,%s,%s" % bbox
        log("  valhalla_build_elevation -b %s" % bounds)
        subprocess.run(
            ["valhalla_build_elevation", "-d", "-b", bounds, "-o", path], check=True
        )
    w, s, e, n = bbox
    files = []
    for lat in range(int(math.floor(s)), int(math.ceil(n))):
        for lon in range(int(math.floor(w)), int(math.ceil(e))):
            name = "%s%02d%s%03d.hgt" % (
                "N" if lat >= 0 else "S",
                abs(lat),
                "E" if lon >= 0 else "W",
                abs(lon),
            )
            candidate = os.path.join(path, name[:3], name)
            if os.path.exists(candidate):
                files.append(candidate)
    log("  %d hgt tiles" % len(files))
    return _build_vrt(sorted(files), os.path.join(workdir, spec["name"] + ".vrt"))


def prepare_raster(spec, bbox, workdir):
    """local files, already in whatever crs they like"""
    files = []
    for pattern in spec["path"] if isinstance(spec["path"], list) else [spec["path"]]:
        found = sorted(globlib.glob(pattern))
        if not found and os.path.exists(pattern):
            found = [pattern]
        files.extend(found)
    log("  %d file(s)" % len(files))
    if len(files) == 1:
        return files[0]
    return _build_vrt(files, os.path.join(workdir, spec["name"] + ".vrt"))


RASTER_EXT = (".tif", ".tiff", ".asc", ".img", ".dt1", ".dt2", ".hgt", ".vrt")
ARCHIVE_RE = re.compile(r"\.(zip|7z|tar|tar\.gz|tgz)(\.\d{3})?$", re.I)


def _archive_stem(name):
    """RGEALTI_x.7z.001 and RGEALTI_x.7z.002 are parts of the same archive"""
    m = ARCHIVE_RE.search(name)
    if m is None:
        return None
    return name[: m.start()]


def _find_rasters(root):
    out = []
    for dirpath, _, filenames in os.walk(root):
        for f in filenames:
            if f.lower().endswith(RASTER_EXT):
                out.append(os.path.join(dirpath, f))
    return sorted(out)


def _extract(archive, dest):
    log("  extract %s" % os.path.basename(archive))
    os.makedirs(dest, exist_ok=True)
    lower = archive.lower()
    if lower.endswith(".zip"):
        import zipfile

        with zipfile.ZipFile(archive) as z:
            z.extractall(dest)
    elif ".tar" in lower or lower.endswith(".tgz"):
        import tarfile

        with tarfile.open(archive) as t:
            t.extractall(dest)
    else:  # 7z, including multi part: give it the first part
        seven = None
        for candidate in ("7zz", "7z", "7za"):
            if (
                subprocess.run(
                    ["which", candidate], capture_output=True
                ).returncode
                == 0
            ):
                seven = candidate
                break
        if seven is None:
            raise SystemExit(
                "extracting %s needs the 7z cli (brew install sevenzip)" % archive
            )
        subprocess.run([seven, "x", "-y", "-o" + dest, archive], check=True)


def _ensure_item(urls, cache):
    """
    Materialize one item of a file list, which is either a plain raster or an
    archive, possibly split in parts. Returns the rasters it provides.

    The extracted folder is looked at first, so deleting the archives once they
    are extracted is fine and does not trigger a new download.
    """
    stem = _archive_stem(os.path.basename(urls[0]))
    if stem is None:
        dest = os.path.join(cache, os.path.basename(urls[0]))
        return [_download(urls[0], dest)]

    extracted = os.path.join(cache, "extracted", stem)
    rasters = _find_rasters(extracted) if os.path.isdir(extracted) else []
    if rasters:
        return rasters

    parts = [_download(u, os.path.join(cache, "archives", os.path.basename(u))) for u in urls]
    _extract(sorted(parts)[0], extracted)
    rasters = _find_rasters(extracted)
    if not rasters:
        raise SystemExit("no raster found in %s after extracting %s" % (extracted, stem))
    return rasters


def _bounds_index(paths, cache):
    """wgs84 bounds of each file, cached: opening 100k rasters is not free"""
    from rasterio.warp import transform_bounds

    index_path = os.path.join(cache, "bounds.json")
    index = {}
    if os.path.exists(index_path):
        index = json.load(open(index_path))
    missing = [p for p in paths if p not in index]
    if missing:
        log("  indexing %d file(s)" % len(missing))
        for p in missing:
            try:
                with rasterio.open(p) as src:
                    if src.crs is None:
                        index[p] = None
                    else:
                        index[p] = list(transform_bounds(src.crs, "EPSG:4326", *src.bounds))
            except Exception:
                index[p] = None
        json.dump(index, open(index_path, "w"))
    return index


def prepare_mapterhorn(spec, bbox, workdir):
    """
    A source of the mapterhorn source-catalog. `path` is either a catalog name,
    a url to a file_list.txt, or a local file_list.txt
    """
    ref = spec.get("path") or spec.get("catalog") or spec["name"]
    cache = spec.get("cache", os.path.join("sources", spec["name"]))
    os.makedirs(cache, exist_ok=True)

    listing = os.path.join(cache, "file_list.txt")
    if os.path.exists(ref):
        listing = ref
    elif "://" in ref:
        _download(ref, listing)
    else:
        _download("%s/%s/file_list.txt" % (CATALOG_URL, ref), listing)
    urls = [u.strip() for u in open(listing) if u.strip()]

    # group the parts of a split archive back together
    groups = {}
    for url in urls:
        stem = _archive_stem(os.path.basename(url)) or os.path.basename(url)
        groups.setdefault(stem, []).append(url)

    named, unnamed = [], []
    for stem, group in sorted(groups.items()):
        tile_bounds = _degree_tile_bounds(group[0])
        if tile_bounds is None:
            unnamed.append(group)
        elif _intersects(tile_bounds, bbox):
            named.append(group)

    if unnamed and not spec.get("allow_full_download"):
        raise SystemExit(
            '%s: %d item(s) carry no coordinates in their filename, so the area '
            "cannot be worked out before downloading. Add "
            '"allow_full_download": true to that source to fetch all of them '
            "(this can be hundreds of GB), or prepare it with the mapterhorn "
            "pipeline and point a `raster` source at the result." % (spec["name"], len(unnamed))
        )
    if unnamed:
        log("  %d item(s) cannot be filtered before download" % len(unnamed))
    log("  %d/%d items to materialize" % (len(named) + len(unnamed), len(groups)))

    files = []
    for group in named + unnamed:
        files.extend(_ensure_item(group, cache))
    log("  %d raster(s)" % len(files))

    if unnamed:
        # now that they are on disk, drop what does not touch the area
        index = _bounds_index(files, cache)
        kept = [p for p in files if index.get(p) is None or _intersects(index[p], bbox)]
        log("  %d raster(s) intersect the area" % len(kept))
        files = kept

    vrt = _build_vrt(files, os.path.join(workdir, spec["name"] + ".vrt"))
    if spec.get("crs"):
        # .asc deliveries carry no projection, mapterhorn sets it in its Justfile
        subprocess.run(["gdal_edit.py", "-a_srs", spec["crs"], vrt], check=True)
    return vrt


PREPARE = {
    "valhalla": prepare_valhalla,
    "raster": prepare_raster,
    "mapterhorn": prepare_mapterhorn,
}


# --------------------------------------------------------------------------
# compositing
# --------------------------------------------------------------------------


def _box_blur(a, radius):
    """separable box blur, run twice so the ramp is a smooth triangular one"""
    if radius < 1:
        return a
    for _ in range(2):
        for axis in (0, 1):
            a = np.swapaxes(a, 0, axis)
            pad = np.pad(a, ((radius + 1, radius), (0, 0)), mode="edge")
            cs = np.cumsum(pad, axis=0, dtype=np.float32)
            a = (cs[2 * radius + 1 :] - cs[: -(2 * radius + 1)]) / (2 * radius + 1)
            a = np.swapaxes(a, 0, axis)
    return a


def source_resolution_m(src):
    """native resolution of a dataset, roughly in metres"""
    res = abs(src.transform.a)
    if src.crs and src.crs.is_geographic:
        return res * 111320.0
    return res


class Source(object):
    """
    A source, opened at the overview level closest to the resolution being asked
    for. Without this a 5m source is read at full resolution to fill a 50m tile
    grid, which is 100x the pixels needed.
    """

    def __init__(self, spec):
        self.spec = spec
        self.path = spec["_path"]
        self._cache = {}
        base = rasterio.open(self.path)
        self.base_res = source_resolution_m(base)
        self.overviews = base.overviews(1)
        self._cache[None] = base

    def at(self, target_res_m):
        level = None
        if self.overviews and target_res_m > self.base_res:
            ratio = target_res_m / self.base_res
            usable = [i for i, f in enumerate(self.overviews) if f <= ratio]
            if usable:
                level = usable[-1]
        if level not in self._cache:
            self._cache[level] = rasterio.open(self.path, overview_level=level)
        return self._cache[level]


def _warp_source(src, shape, dst_transform, nodata_out):
    out = np.full(shape, nodata_out, dtype=np.float32)
    reproject(
        rasterio.band(src, 1),
        out,
        dst_transform=dst_transform,
        dst_crs="EPSG:3857",
        dst_nodata=nodata_out,
        resampling=Resampling.bilinear,
    )
    return out


def composite(sources, shape, dst_transform, blur_px, nodata_elev, target_res_m):
    """
    Stack the sources lowest priority first. Each one fades in over blur_px
    pixels, and the fade is truncated to where that source actually has data:
    past its edge there is nothing to fade towards.
    """
    SENTINEL = np.float32(-3.0e38)
    out = None
    for source in sources:
        spec = source.spec
        arr = _warp_source(source.at(target_res_m), shape, dst_transform, SENTINEL)
        valid = arr != SENTINEL
        if not valid.any():
            continue
        clamp = spec.get("clamp_min")
        if clamp is not None:
            arr = np.where(valid, np.maximum(arr, np.float32(clamp)), arr)
        if out is None:
            out = np.where(valid, arr, np.float32(nodata_elev))
            continue
        weight = valid.astype(np.float32)
        if blur_px >= 1:
            weight = _box_blur(weight, int(blur_px)) * weight
        out = out * (1.0 - weight) + np.where(valid, arr, out) * weight
    if out is None:
        out = np.full(shape, nodata_elev, dtype=np.float32)
    return out


# --------------------------------------------------------------------------
# tiles
# --------------------------------------------------------------------------


def tiles_for_zoom(geom, bbox, zoom, tile_buffer):
    """every tile whose extent, grown by tile_buffer tiles, touches the shape"""
    w, s, e, n = bbox
    max_index = 2 ** zoom - 1
    ul = mercantile.tile(w + 1e-10, min(n - 1e-10, 85.0), zoom)
    lr = mercantile.tile(e - 1e-10, max(s + 1e-10, -85.0), zoom)
    out = []
    for x in range(max(0, ul.x - tile_buffer), min(max_index, lr.x + tile_buffer) + 1):
        for y in range(
            max(0, ul.y - tile_buffer), min(max_index, lr.y + tile_buffer) + 1
        ):
            tw, ts, te, tn = mercantile.bounds(x, y, zoom)
            if tile_buffer:
                dx, dy = (te - tw) * tile_buffer, (tn - ts) * tile_buffer
                tw, te, ts, tn = tw - dx, te + dx, ts - dy, tn + dy
            if geom is None or geometry.box(tw, ts, te, tn).intersects(geom):
                out.append((x, y, zoom))
    return out


def round_digits_for(zoom, maxzoom, round_digits, max_round_digits):
    return min(round_digits + (maxzoom - zoom), max_round_digits)


def encode(elev, base, interval, round_digits, fmt):
    rgb = data_to_rgb(elev.astype(np.float64), base, interval, round_digits)
    im = Image.fromarray(np.rollaxis(rgb, 0, 3))
    buf = io.BytesIO()
    if fmt == "webp":
        im.save(buf, format="webp", lossless=True)
    else:
        im.save(buf, format="png")
    return buf.getvalue()


def decode(blob, base, interval):
    arr = np.asarray(Image.open(io.BytesIO(blob)).convert("RGB"), dtype=np.float64)
    r, g, b = arr[:, :, 0], arr[:, :, 1], arr[:, :, 2]
    return base + (r * 65536.0 + g * 256.0 + b) * interval


def _init_worker(specs, args):
    WORKER["sources"] = [Source(s) for s in specs]
    WORKER["args"] = args


def warp_block(sources, a, mx, my, mz, zoom):
    """composite the area of tile mx/my/mz, at the resolution of `zoom`"""
    tiles_side = 2 ** (zoom - mz)
    size = tiles_side * a["tile_size"]
    halo = a["halo"]

    west, south, east, north = mercantile.xy_bounds(mx, my, mz)
    res = (east - west) / size
    full = size + 2 * halo
    dst_transform = riotransform.from_origin(
        west - halo * res, north + halo * res, res, res
    )

    # a rough latitude correction: web mercator metres are not ground metres
    lat = math.radians(mercantile.bounds(mx, my, mz).north)
    ground_res = res * max(0.1, math.cos(lat))
    blur_px = a["blur"] / ground_res if a["blur"] else 0

    elev = composite(
        sources, (full, full), dst_transform, blur_px, a["nodata_elev"], ground_res
    )
    if halo:
        elev = elev[halo : halo + size, halo : halo + size]
    return elev


def _macro_worker(job):
    """warp one macrotile once, then slice it into max zoom tiles"""
    mx, my, mz = job
    a = WORKER["args"]
    tiles_side = 2 ** (a["maxzoom"] - mz)
    elev = warp_block(WORKER["sources"], a, mx, my, mz, a["maxzoom"])

    base, interval = ENCODINGS[a["encoding"]]
    rd = round_digits_for(a["maxzoom"], a["maxzoom"], a["round_digits"], a["max_round_digits"])
    out = []
    for ty in range(tiles_side):
        for tx in range(tiles_side):
            sub = elev[
                ty * a["tile_size"] : (ty + 1) * a["tile_size"],
                tx * a["tile_size"] : (tx + 1) * a["tile_size"],
            ]
            x = mx * tiles_side + tx
            y = my * tiles_side + ty
            if (x, y, a["maxzoom"]) not in a["wanted"]:
                continue
            out.append(((x, y, a["maxzoom"]), encode(sub, base, interval, rd, a["format"])))
    return out


# --------------------------------------------------------------------------
# mbtiles
# --------------------------------------------------------------------------


def zxy_to_id(z, x, y):
    return int((1 - pow(4, z)) / (1 - 4) + pow(2, z) * y + x)


class MBTiles(object):
    def __init__(self, path):
        if os.path.exists(path):
            os.unlink(path)
        self.conn = sqlite3.connect(path)
        cur = self.conn.cursor()
        cur.execute("PRAGMA synchronous=OFF")
        cur.execute("PRAGMA journal_mode=MEMORY")
        cur.execute(
            "CREATE TABLE tiles_data (tile_data_id integer primary key, tile_data blob);"
        )
        cur.execute(
            "CREATE TABLE tiles_shallow (zoom_level integer, tile_column integer, "
            "tile_row integer, tile_data_id integer, "
            "primary key(zoom_level,tile_column,tile_row)) without rowid;"
        )
        cur.execute(
            "CREATE VIEW tiles AS SELECT tiles_shallow.zoom_level as zoom_level, "
            "tiles_shallow.tile_column as tile_column, "
            "tiles_shallow.tile_row as tile_row, tiles_data.tile_data as tile_data "
            "FROM tiles_shallow JOIN tiles_data on "
            "tiles_shallow.tile_data_id = tiles_data.tile_data_id"
        )
        cur.execute("CREATE TABLE metadata (name text, value text);")
        self.conn.commit()

    def put(self, tile, blob):
        x, y, z = tile
        tile_id = zxy_to_id(z, x, y)
        flipped = int(math.pow(2, z)) - y - 1
        cur = self.conn.cursor()
        cur.execute(
            "INSERT OR REPLACE INTO tiles_data (tile_data_id, tile_data) VALUES (?, ?)",
            (tile_id, sqlite3.Binary(blob)),
        )
        cur.execute(
            "INSERT OR REPLACE INTO tiles_shallow "
            "(zoom_level, tile_column, tile_row, tile_data_id) VALUES (?, ?, ?, ?)",
            (z, x, flipped, tile_id),
        )

    def get(self, tile):
        x, y, z = tile
        flipped = int(math.pow(2, z)) - y - 1
        cur = self.conn.cursor()
        cur.execute(
            "SELECT tile_data FROM tiles WHERE zoom_level=? AND tile_column=? "
            "AND tile_row=?",
            (z, x, flipped),
        )
        row = cur.fetchone()
        return row[0] if row else None

    def delete(self, tile):
        x, y, z = tile
        cur = self.conn.cursor()
        cur.execute(
            "DELETE FROM tiles_shallow WHERE zoom_level=? AND tile_column=? "
            "AND tile_row=?",
            (z, x, int(math.pow(2, z)) - y - 1),
        )
        cur.execute("DELETE FROM tiles_data WHERE tile_data_id=?", (zxy_to_id(z, x, y),))

    def vacuum(self):
        self.conn.commit()
        self.conn.execute("VACUUM")

    def metadata(self, items):
        cur = self.conn.cursor()
        for k, v in items.items():
            cur.execute("INSERT INTO metadata (name, value) VALUES (?, ?)", (k, str(v)))
        self.conn.commit()

    def commit(self):
        self.conn.commit()

    def close(self):
        self.conn.commit()
        self.conn.close()


# --------------------------------------------------------------------------


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--sources", required=True, help="json file describing the sources")
    p.add_argument("--poly-shape", help="osmosis .poly limiting the tiles")
    p.add_argument("--bounds", help="w,s,e,n instead of --poly-shape")
    p.add_argument("--minzoom", type=int, default=5)
    p.add_argument("--maxzoom", type=int, default=12)
    p.add_argument("--round-digits", type=int, default=0)
    p.add_argument("--max-round-digits", type=int, default=0)
    p.add_argument("-f", "--format", choices=("webp", "png"), default="webp")
    p.add_argument("--encoding", choices=tuple(ENCODINGS), default="mapbox")
    p.add_argument("--blur", type=float, default=1000.0, help="source fade, in metres")
    p.add_argument(
        "--tile-buffer",
        type=int,
        default=0,
        help="ring of extra tiles to write around the shape. 3d terrain renderers "
        "backfill the 1px border of a dem tile from its neighbours, so a ring "
        "removes the seam at the edge of the covered area. The tiles needed to "
        "average the lower zooms are worked out on their own, this does not "
        "change them",
    )
    p.add_argument("--tile-size", type=int, default=512)
    p.add_argument(
        "--macro-levels",
        type=int,
        default=2,
        help="warp 2^N x 2^N max zoom tiles at once (default 2, so 4x4 tiles)",
    )
    p.add_argument("--nodata-elevation", type=float, default=-10.0)
    p.add_argument(
        "--allow-full-download",
        action="store_true",
        help="allow sources whose file names carry no coordinates to be fetched "
        "whole, when the area cannot be worked out before downloading",
    )
    p.add_argument("-j", "--workers", type=int, default=8)
    p.add_argument("--workdir", default=".terrain_rgb")
    p.add_argument("-o", "--output", required=True)
    a = p.parse_args()

    if a.max_round_digits < a.round_digits:
        a.max_round_digits = a.round_digits

    geom = None
    if a.poly_shape:
        geom = parse_poly(a.poly_shape)
        bbox = geom.bounds
    elif a.bounds:
        bbox = tuple(float(v) for v in a.bounds.split(","))
    else:
        raise SystemExit("--poly-shape or --bounds is required")
    log("area %s" % (tuple(round(v, 4) for v in bbox),))

    os.makedirs(a.workdir, exist_ok=True)
    specs = json.load(open(a.sources))
    specs = specs["sources"] if isinstance(specs, dict) else specs

    # what ends up in the mbtiles
    store = {
        z: set(tiles_for_zoom(geom, bbox, z, a.tile_buffer))
        for z in range(a.minzoom, a.maxzoom + 1)
    }
    # what has to be produced: the stored tiles, plus the children of the tiles
    # stored one zoom below, so that every one of them can be averaged from its
    # own 4 children instead of being warped again. these extras are not written
    wanted = {}
    for z in range(a.minzoom, a.maxzoom + 1):
        needed = set(store[z])
        if z > a.minzoom:
            for px, py, _ in store[z - 1]:
                for dy in (0, 1):
                    for dx in (0, 1):
                        needed.add((px * 2 + dx, py * 2 + dy, z))
        wanted[z] = sorted(needed)
    for z in sorted(wanted):
        extra = len(wanted[z]) - len(store[z])
        log("z%-2d %d tiles%s" % (z, len(store[z]), " (+%d to average" % extra + " the zoom below)" if extra else ""))

    # ---- max zoom, warped once per macrotile
    macro_z = max(0, a.maxzoom - a.macro_levels)
    side = 2 ** (a.maxzoom - macro_z)
    macro_jobs = sorted({(x // side, y // side, macro_z) for x, y, _ in wanted[a.maxzoom]})

    blur_px_guess = 0
    max_res = (2 * math.pi * 6378137.0) / (2 ** a.maxzoom * a.tile_size)
    if a.blur:
        blur_px_guess = int(a.blur / max_res)
    halo = 2 * blur_px_guess + 4 if a.blur else 0

    # the sources have to cover every pixel that gets read, which is the macrotile
    # blocks plus their halo, not the shape. clipping them to the shape leaves the
    # tiles on the boundary composited from incomplete data, and they then differ
    # from the same tile built as part of a neighbouring area
    margin = (halo + a.tile_size) * max_res
    mercator = [mercantile.xy_bounds(*job) for job in macro_jobs]
    west, south = mercantile.lnglat(
        min(b.left for b in mercator) - margin, min(b.bottom for b in mercator) - margin
    )
    east, north = mercantile.lnglat(
        max(b.right for b in mercator) + margin, max(b.top for b in mercator) + margin
    )
    source_bbox = (west, south, east, north)
    log("sources cover %s" % (tuple(round(v, 4) for v in source_bbox),))

    for spec in specs:
        if a.allow_full_download:
            spec["allow_full_download"] = True
        log("source %s (%s)" % (spec["name"], spec.get("type", "raster")))
        prepare = PREPARE[spec.get("type", "raster")]
        spec["_path"] = prepare(spec, source_bbox, a.workdir)

    db = MBTiles(a.output)
    log(
        "z%d: %d tiles in %d macrotiles of %dpx (round-digits %d)"
        % (
            a.maxzoom,
            len(wanted[a.maxzoom]),
            len(macro_jobs),
            side * a.tile_size,
            a.round_digits,
        )
    )
    if a.max_round_digits == a.round_digits and a.maxzoom > a.minzoom:
        log(
            "  note: --max-round-digits is %d, so every zoom quantizes the same. "
            "raise it to let the lower zooms compress harder" % a.max_round_digits
        )

    worker_args = {
        "maxzoom": a.maxzoom,
        "tile_size": a.tile_size,
        "halo": halo,
        "blur": a.blur,
        "encoding": a.encoding,
        "format": a.format,
        "round_digits": a.round_digits,
        "max_round_digits": a.max_round_digits,
        "nodata_elev": a.nodata_elevation,
        "wanted": set(wanted[a.maxzoom]),
    }

    done = 0
    if a.workers > 1:
        pool = Pool(a.workers, _init_worker, (specs, worker_args))
        results = pool.imap_unordered(_macro_worker, macro_jobs)
    else:
        _init_worker(specs, worker_args)
        results = (_macro_worker(j) for j in macro_jobs)
    for batch in results:
        for tile, blob in batch:
            db.put(tile, blob)
        done += 1
        if done % 20 == 0:
            db.commit()
            log("  %d/%d macrotiles" % (done, len(macro_jobs)))
    db.commit()
    if a.workers > 1:
        pool.close()
        pool.join()

    # ---- lower zooms, 2x2 average of the tiles already produced
    base, interval = ENCODINGS[a.encoding]
    main_sources = [Source(s) for s in specs]
    for z in range(a.maxzoom - 1, a.minzoom - 1, -1):
        rd = round_digits_for(z, a.maxzoom, a.round_digits, a.max_round_digits)
        made = 0
        rewarped = 0
        for x, y, _ in wanted[z]:
            quad = np.empty((a.tile_size * 2, a.tile_size * 2), dtype=np.float64)
            children = [
                db.get((x * 2 + dx, y * 2 + dy, z + 1)) for dy in (0, 1) for dx in (0, 1)
            ]
            if all(c is not None for c in children):
                for i, blob in enumerate(children):
                    dy, dx = divmod(i, 2)
                    quad[
                        dy * a.tile_size : (dy + 1) * a.tile_size,
                        dx * a.tile_size : (dx + 1) * a.tile_size,
                    ] = decode(blob, base, interval)
                elev = 0.25 * (
                    quad[0::2, 0::2]
                    + quad[1::2, 0::2]
                    + quad[0::2, 1::2]
                    + quad[1::2, 1::2]
                )
            else:
                # a tile of the buffer ring can have children outside of the
                # selection of the zoom below. warping it straight from the dem
                # is cheaper than widening every zoom down to the max one
                elev = warp_block(main_sources, worker_args, x, y, z, z)
                rewarped += 1
            db.put((x, y, z), encode(elev, base, interval, rd, a.format))
            made += 1
        db.commit()
        log(
            "z%-2d %d tiles (round-digits %d, %d re-warped)"
            % (z, made, rd, rewarped)
        )

    # the extras only existed to average the zoom below, they are not output
    dropped = 0
    for z in range(a.minzoom, a.maxzoom + 1):
        for tile in wanted[z]:
            if tile not in store[z]:
                db.delete(tile)
                dropped += 1
    if dropped:
        log("dropped %d tiles only used to average the lower zooms" % dropped)
        db.vacuum()

    db.metadata(
        {
            "name": os.path.splitext(os.path.basename(a.output))[0],
            "format": a.format,
            "type": "baselayer",
            "version": "1",
            "description": "%s terrain rgb" % a.encoding,
            "encoding": a.encoding,
            "minzoom": a.minzoom,
            "maxzoom": a.maxzoom,
            "bounds": ",".join(str(round(v, 6)) for v in bbox),
        }
    )
    db.close()
    log("wrote %s" % a.output)


if __name__ == "__main__":
    main()
