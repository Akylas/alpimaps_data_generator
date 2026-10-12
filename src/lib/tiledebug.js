// Debug helpers: tile boundaries, and dumping a tile's contents as JSON.

import { VectorTile } from "@mapbox/vector-tile";
// pbf 5 exports readers and writers by name; there is no default export
import { PbfReader } from "pbf";

/** Web-mercator tile covering a point at a zoom. */
export function pointToTile(lon, lat, zoom) {
  const n = 2 ** zoom;
  const x = Math.floor(((lon + 180) / 360) * n);
  const rad = (lat * Math.PI) / 180;
  const y = Math.floor(((1 - Math.log(Math.tan(rad) + 1 / Math.cos(rad)) / Math.PI) / 2) * n);
  return { z: zoom, x: Math.min(n - 1, Math.max(0, x)), y: Math.min(n - 1, Math.max(0, y)) };
}

function tileBounds(z, x, y) {
  const n = 2 ** z;
  const lon = (i) => (i / n) * 360 - 180;
  const lat = (j) => {
    const t = Math.PI - (2 * Math.PI * j) / n;
    return (180 / Math.PI) * Math.atan(0.5 * (Math.exp(t) - Math.exp(-t)));
  };
  return [lon(x), lat(y + 1), lon(x + 1), lat(y)];
}

/**
 * Grid of the tiles currently in view, as outlines plus a label per tile.
 *
 * Drawn from the map's own bounds rather than from any source, so it shows where tile edges are
 * even for a raster-DEM layer whose seams are the thing being looked for.
 */
export function tileGrid(map, maxTiles = 400) {
  const zoom = Math.floor(map.getZoom());
  const bounds = map.getBounds();
  const min = pointToTile(bounds.getWest(), bounds.getNorth(), zoom);
  const max = pointToTile(bounds.getEast(), bounds.getSouth(), zoom);
  const features = [];

  for (let x = min.x; x <= max.x; x++) {
    for (let y = min.y; y <= max.y; y++) {
      if (features.length > maxTiles * 2) break;
      const [w, s, e, n] = tileBounds(zoom, x, y);
      features.push({
        type: "Feature",
        geometry: {
          type: "LineString",
          coordinates: [[w, n], [e, n], [e, s], [w, s], [w, n]],
        },
        properties: { label: "" },
      });
      features.push({
        type: "Feature",
        geometry: { type: "Point", coordinates: [(w + e) / 2, (s + n) / 2] },
        properties: { label: `${zoom}/${x}/${y}` },
      });
    }
  }
  return { type: "FeatureCollection", features };
}

/** Decode an MVT tile into plain JSON. */
export function decodeMvt(buffer) {
  const tile = new VectorTile(new PbfReader(new Uint8Array(buffer)));
  const layers = {};
  for (const name of Object.keys(tile.layers)) {
    const layer = tile.layers[name];
    const features = [];
    for (let i = 0; i < layer.length; i++) {
      const feature = layer.feature(i);
      features.push({
        id: feature.id,
        type: ["Unknown", "Point", "LineString", "Polygon"][feature.type] ?? feature.type,
        properties: feature.properties,
      });
    }
    layers[name] = { extent: layer.extent, count: layer.length, features };
  }
  return layers;
}

/** Decode an MLT tile into the same shape. */
export async function decodeMlt(buffer) {
  const { decodeTile } = await import("@maplibre/mlt");
  const layers = {};
  for (const table of decodeTile(new Uint8Array(buffer))) {
    const features = table.getFeatures().map((f) => ({
      // ids past 2^53 arrive as BigInt, which JSON.stringify refuses outright
      id: typeof f.id === "bigint" ? f.id.toString() : f.id,
      type: f.geometry?.type,
      properties: f.properties,
    }));
    layers[table.name] = { extent: table.extent, count: features.length, features };
  }
  return layers;
}

/**
 * Fetch one tile from the local server and decode it.
 *
 * `limit` caps how many features per layer are returned: a z14 basemap tile carries tens of
 * thousands, and serialising all of them locks the window for seconds.
 */
export async function fetchTileJson(base, area, file, { z, x, y }, encoding = "mvt", limit = 50) {
  const response = await fetch(`${base}/tiles/${area}/${file}/${z}/${x}/${y}`);
  if (response.status === 204) return { empty: true, z, x, y };
  if (!response.ok) throw new Error(`tile ${z}/${x}/${y}: HTTP ${response.status}`);
  const buffer = await response.arrayBuffer();

  const layers = encoding === "mlt" ? await decodeMlt(buffer) : decodeMvt(buffer);
  const trimmed = {};
  for (const [name, layer] of Object.entries(layers)) {
    trimmed[name] = {
      extent: layer.extent,
      count: layer.count,
      features: layer.features.slice(0, limit),
      truncated: layer.count > limit ? layer.count - limit : 0,
    };
  }
  return { z, x, y, bytes: buffer.byteLength, layers: trimmed };
}
