import click

import mercantile
import itertools

from shapely import geometry, wkt, ops

def _tile_range(min_tile, max_tile):
    """
    Given a min and max tile, return an iterator of
    all combinations of this tile range

    Parameters
    -----------
    min_tile: list
        [x, y, z] of minimun tile
    max_tile:
        [x, y, z] of minimun tile

    Returns
    --------
    tiles: iterator
        iterator of [x, y, z] tiles
    """
    min_x, min_y, _ = min_tile
    max_x, max_y, _ = max_tile

    return itertools.product(range(min_x, max_x + 1), range(min_y, max_y + 1))


def parse_poly(filepath):
    """Parse an Osmosis polygon filter file.

    Accept a sequence of lines from a polygon file, return a shapely.geometry.MultiPolygon object.

    http://wiki.openstreetmap.org/wiki/Osmosis/Polygon_Filter_File_Format
    """

    file1 = open(filepath, "r")
    lines = file1.readlines()
    in_ring = False
    coords = []

    for (index, line) in enumerate(lines):
        if index == 0:
            # first line is junk.
            continue

        elif index == 1:
            # second line is the first polygon ring.
            coords.append([[], []])
            ring = coords[-1][0]
            in_ring = True

        elif in_ring and line.strip() == "END":
            # we are at the end of a ring, perhaps with more to come.
            in_ring = False

        elif in_ring:
            # we are in a ring and picking up new coordinates.
            ring.append(list(map(float, line.split())))

        elif not in_ring and line.strip() == "END":
            # we are at the end of the whole polygon.
            break

        elif not in_ring and line.startswith("!"):
            # we are at the start of a polygon part hole.
            coords[-1][1].append([])
            ring = coords[-1][1][-1]
            in_ring = True

        elif not in_ring:
            # we are at the start of a polygon part.
            coords.append([[], []])
            ring = coords[-1][0]
            in_ring = True

    return geometry.MultiPolygon(coords)

def get_tilesBounds(geom, minz, maxz):
    EPSILON = 1.0e-10
    w, s, e, n = geom.bounds
    w += EPSILON
    s += EPSILON
    e -= EPSILON
    n -= EPSILON

    minx=10000
    miny=10000
    maxx=-10000
    maxy=-10000

    for z in range(minz, maxz + 1):
        for x, y in _tile_range(mercantile.tile(w, n, z), mercantile.tile(e, s, z)):
            tw, ts, te, tn = mercantile.bounds(x, y, z)
            tileGeometry = geometry.Polygon(
                [[tw, ts], [tw, tn], [te, tn], [te, ts], [tw, ts]]
            )
            if tileGeometry.intersects(geom):
                minx=min(minx, tw)
                miny=min(miny, ts)
                maxx=max(maxx, te)
                maxy=max(maxy, tn)
    return (minx, miny, maxx, maxy)

@click.command()
@click.option(
    "--bounds",
    type=str,
    default=None,
    help="bounds to compute tiles bounds from '{w},{s},{e},{n}'",
)
@click.option(
    '-p', 
    "--poly-shape",
    type=str,
    default=None,
    help="generate bounds from poly shape",
)
@click.option(
    "--minzoom",
    type=int,
    default=0,
    help="Minimum zoom to compute tiles bounds",
)
@click.option(
    "--maxzoom",
    type=int,
    default=18,
    help="Maximum zoom to compute tiles bounds",
)
@click.pass_context
def main(
    ctx,
    maxzoom,
    minzoom,
    bounds,
    poly_shape
):
    if poly_shape is not None:
        geom = parse_poly(poly_shape)
        tile_bounds = get_tilesBounds(geom, minzoom, maxzoom)
        print(','.join(map(str, tile_bounds)))
        return tile_bounds
    elif bounds is not None:
        w, s, e, n = bounds.split(", ")
        geom=geometry.Polygon(
            [[w, s], [w, n], [e, n], [e, s], [w, s]]
        )
        tile_bounds = get_tilesBounds(geom, minzoom, maxzoom)
        print(','.join(map(str, tile_bounds)))
        return tile_bounds


if __name__ == '__main__':

    main()


   
