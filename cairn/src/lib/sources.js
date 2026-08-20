// Source and layer management for the map view.
//
// Adapted from mbview-rs (`src/lib/sources.ts`), which already solved this properly. The parts
// worth keeping verbatim are the *derived* visibility model and the ordering trick; both are
// noted where they appear.

import { randomColor } from "randomcolor";

/** Prefix on every layer this app creates, so a backdrop's own layers stay distinguishable. */
export const OWN_PREFIX = "___";

export const GEOM_KINDS = ["polygons", "lines", "points", "rasters"];

/**
 * How a terrain-RGB archive is drawn.
 *
 * `hillshade` is the useful view and `raster` is the honest one: it paints the encoded bytes
 * directly, so quantisation banding and the seams between sources are visible as themselves
 * rather than as shading. `terrain3d` drapes the DEM, which is how tile edges give themselves
 * away - a mismatched edge is a cliff.
 */
export const TERRAIN_MODES = ["hillshade", "raster", "terrain3d"];

export function layerIdPrefix(sourceId, layerId) {
  return `${OWN_PREFIX}${sourceId}${OWN_PREFIX}${layerId}`;
}

export function isOwnLayer(id) {
  return id.startsWith(OWN_PREFIX);
}

function emptyMapLayers() {
  return { polygons: [], lines: [], points: [], rasters: [] };
}

/**
 * A stable colour per source-layer name, nudged by what the name suggests so water reads blue
 * and roads read orange across every file opened.
 */
export function brightColor(layerId, alpha) {
  let luminosity = "bright";
  let hue = null;
  if (/water|ocean|lake|sea|river/.test(layerId)) hue = "blue";
  if (/state|country|place|boundary/.test(layerId)) hue = "pink";
  if (/road|highway|transport|route/.test(layerId)) hue = "orange";
  if (/contour|landuse/.test(layerId)) hue = "yellow";
  if (/wood|forest|park|landcover/.test(layerId)) hue = "green";
  if (/building/.test(layerId)) {
    luminosity = "light";
    hue = "monochrome";
  }
  const rgb = randomColor({ luminosity, hue, seed: layerId, format: "rgbArray" });
  return `rgba(${[...rgb, alpha ?? 1].join(", ")})`;
}

/** Base opacity per layer role, before the source's own opacity scales it. */
const BASE_OPACITY = { fill: 0.1, outline: 0.75, line: 0.75, circle: 0.75, raster: 1 };

/**
 * Build a source entry from a catalog artifact plus its TileJSON.
 *
 * `kind` drives the maplibre source type: terrain RGB becomes `raster-dem` with a hillshade
 * layer, which is why classifying a `_hillshade`-named archive as a picture would render flat
 * imagery instead of relief.
 */
export function makeSource(artifact, tilejson, base, prefs = {}) {
  const id = `${artifact.area}__${artifact.file_name}`.replace(/[^A-Za-z0-9_]/g, "_");
  const vector = (tilejson.vector_layers ?? []).length > 0;
  const terrain = artifact.kind === "terrain_rgb";

  const layers = vector
    ? tilejson.vector_layers
        .map((l) => ({
          id: l.id,
          color: brightColor(l.id),
          visible: prefs.layers?.[l.id] ?? true,
          minzoom: l.minzoom,
          maxzoom: l.maxzoom,
          fields: Object.keys(l.fields ?? {}),
          mapLayers: emptyMapLayers(),
        }))
        .sort((a, b) => a.id.localeCompare(b.id))
    : [{ id: artifact.file_name, color: "transparent", visible: true, mapLayers: emptyMapLayers() }];

  return {
    id,
    artifact,
    file: artifact.file_name,
    area: artifact.area,
    vector,
    terrain,
    // `mvt` vs `mlt` is the tile container; distinct from the DEM `encoding` below
    tileEncoding: tilejson.tileEncoding ?? "mvt",
    demEncoding: artifact.encoding ?? tilejson.encoding ?? null,
    url: `${base}/tilejson/${artifact.area}/${artifact.file_name}`,
    bounds: tilejson.bounds,
    minzoom: tilejson.minzoom,
    maxzoom: tilejson.maxzoom,
    tileSize: tilejson.tileSize ?? 512,
    visible: prefs.visible ?? true,
    opacity: prefs.opacity ?? 1,
    terrainMode: prefs.terrainMode ?? "hillshade",
    layers,
  };
}

/** Turn every layer of a source on or off at once. */
export function setAllLayers(source, visible) {
  for (const layer of source.layers) {
    layer.visible = visible;
  }
}

export function layerSummary(source) {
  const on = source.layers.filter((l) => l.visible).length;
  return { on, total: source.layers.length };
}

export function addSourceToMap(map, source) {
  if (map.getSource(source.id)) removeSourceFromMap(map, source);

  if (source.vector) {
    map.addSource(source.id, {
      type: "vector",
      url: source.url,
      // MLT is not auto-detected: without this the tiles arrive and fail to decode
      encoding: source.tileEncoding,
    });
    for (const layer of source.layers) {
      const prefix = layerIdPrefix(source.id, layer.id);
      layer.mapLayers = emptyMapLayers();
      const common = { source: source.id, "source-layer": layer.id };

      const polygonId = `${prefix}-polygons`;
      map.addLayer({
        ...common, id: polygonId, type: "fill", filter: ["==", "$type", "Polygon"],
        paint: { "fill-opacity": BASE_OPACITY.fill, "fill-color": layer.color },
      });
      const outlineId = `${polygonId}-outline`;
      map.addLayer({
        ...common, id: outlineId, type: "line", filter: ["==", "$type", "Polygon"],
        layout: { "line-join": "round", "line-cap": "round" },
        paint: { "line-color": layer.color, "line-width": 1, "line-opacity": BASE_OPACITY.outline },
      });
      layer.mapLayers.polygons.push(polygonId, outlineId);

      const lineId = `${prefix}-lines`;
      map.addLayer({
        ...common, id: lineId, type: "line", filter: ["==", "$type", "LineString"],
        layout: { "line-join": "round", "line-cap": "round" },
        paint: { "line-color": layer.color, "line-width": 1, "line-opacity": BASE_OPACITY.line },
      });
      layer.mapLayers.lines.push(lineId);

      const pointId = `${prefix}-points`;
      map.addLayer({
        ...common, id: pointId, type: "circle", filter: ["==", "$type", "Point"],
        paint: { "circle-color": layer.color, "circle-radius": 2.5, "circle-opacity": BASE_OPACITY.circle },
      });
      layer.mapLayers.points.push(pointId);
    }
    return;
  }

  const layerId = `${OWN_PREFIX}${source.id}-raster`;
  if (source.terrain) {
    if (source.terrainMode === "raster") {
      // painted as ordinary imagery: what the encoded bytes actually look like
      map.addSource(source.id, { type: "raster", url: source.url, tileSize: source.tileSize });
      map.addLayer({ id: layerId, type: "raster", source: source.id });
    } else {
      map.addSource(source.id, {
        type: "raster-dem",
        url: source.url,
        // terrarium and mapbox pack heights differently; the wrong one renders plausible nonsense
        encoding: source.demEncoding ?? "terrarium",
        tileSize: source.tileSize,
      });
      map.addLayer({ id: layerId, type: "hillshade", source: source.id });
      if (source.terrainMode === "terrain3d") {
        map.setTerrain({ source: source.id, exaggeration: 1.3 });
      }
    }
  } else {
    map.addSource(source.id, { type: "raster", url: source.url, tileSize: source.tileSize });
    map.addLayer({ id: layerId, type: "raster", source: source.id });
  }
  source.layers[0].mapLayers = { ...emptyMapLayers(), rasters: [layerId] };
}

export function removeSourceFromMap(map, source) {
  if (!map || !map.style) return;
  // the terrain reference has to go first, or removing the source it points at throws
  if (source.terrain && map.getTerrain?.()?.source === source.id) {
    map.setTerrain(null);
  }
  for (const id of mapLayerIds(source)) {
    if (map.getLayer(id)) map.removeLayer(id);
  }
  if (map.getSource(source.id)) map.removeSource(source.id);
}

/** Every maplibre layer id belonging to a source, in painter order. */
export function mapLayerIds(source) {
  const ids = [];
  for (const layer of source.layers) {
    ids.push(...layer.mapLayers.rasters, ...layer.mapLayers.polygons,
             ...layer.mapLayers.lines, ...layer.mapLayers.points);
  }
  return ids;
}

function kindAllowed(kind, filter) {
  if (kind === "rasters") return true;
  return filter === "all" || filter === kind;
}

/**
 * Recompute visibility for one source from scratch.
 *
 * Derived, never toggled. The source flag, the per-layer flag and the geometry filter are three
 * independent inputs; anything that tries to remember their combination drifts the moment one of
 * them changes behind its back.
 */
export function applyVisibility(map, source, filter = "all") {
  if (!map || !map.style) return;
  for (const layer of source.layers) {
    for (const kind of GEOM_KINDS) {
      const on = source.visible && layer.visible && kindAllowed(kind, filter);
      for (const id of layer.mapLayers[kind]) {
        if (map.getLayer(id)) map.setLayoutProperty(id, "visibility", on ? "visible" : "none");
      }
    }
  }
}

export function applyOpacity(map, source) {
  if (!map || !map.style) return;
  const factor = source.opacity;
  for (const layer of source.layers) {
    for (const id of layer.mapLayers.polygons) {
      if (!map.getLayer(id)) continue;
      if (id.endsWith("-outline")) map.setPaintProperty(id, "line-opacity", BASE_OPACITY.outline * factor);
      else map.setPaintProperty(id, "fill-opacity", BASE_OPACITY.fill * factor);
    }
    for (const id of layer.mapLayers.lines) {
      if (map.getLayer(id)) map.setPaintProperty(id, "line-opacity", BASE_OPACITY.line * factor);
    }
    for (const id of layer.mapLayers.points) {
      if (map.getLayer(id)) map.setPaintProperty(id, "circle-opacity", BASE_OPACITY.circle * factor);
    }
    for (const id of layer.mapLayers.rasters) {
      const l = map.getLayer(id);
      if (!l) continue;
      // hillshade has no opacity property of that name
      if (l.type === "raster") map.setPaintProperty(id, "raster-opacity", factor);
      else if (l.type === "hillshade") map.setPaintProperty(id, "hillshade-exaggeration", factor);
    }
  }
}

/**
 * Restack so the panel reads top-to-bottom: `sources[0]` draws above `sources[1]`.
 *
 * Walking bottom-up and moving each layer to the very top leaves any backdrop underneath without
 * needing to know a single one of its layer names.
 */
export function applyOrder(map, sources) {
  if (!map || !map.style) return;
  for (let i = sources.length - 1; i >= 0; i--) {
    for (const id of mapLayerIds(sources[i])) {
      if (map.getLayer(id)) map.moveLayer(id);
    }
  }
}

/** Bounds clamped to something maplibre will accept. */
export function boundsOf(source) {
  const b = source.bounds;
  if (!b || b.length < 4) return null;
  const [w, s, e, n] = b;
  if ([w, s, e, n].some((v) => typeof v !== "number" || Number.isNaN(v))) return null;
  return [Math.max(-180, w), Math.max(-85, s), Math.min(180, e), Math.min(85, n)];
}

/** Raster backdrops usable underneath everything. */
export const BACKDROPS = {
  osm: {
    label: "OpenStreetMap",
    tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"],
    tileSize: 256,
    attribution: "© OpenStreetMap contributors",
  },
  ign_plan: {
    label: "IGN Plan",
    tiles: ["https://data.geopf.fr/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=GEOGRAPHICALGRIDSYSTEMS.PLANIGNV2&STYLE=normal&TILEMATRIXSET=PM&FORMAT=image/png&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}"],
    tileSize: 256,
    attribution: "© IGN",
  },
  ign_ortho: {
    label: "IGN Orthophoto",
    tiles: ["https://data.geopf.fr/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=ORTHOIMAGERY.ORTHOPHOTOS&STYLE=normal&TILEMATRIXSET=PM&FORMAT=image/jpeg&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}"],
    tileSize: 256,
    attribution: "© IGN",
  },
};

export function baseStyle(backdrop) {
  const style = {
    version: 8,
    sources: {},
    layers: [{ id: "background", type: "background", paint: { "background-color": "#12151a" } }],
  };
  const b = BACKDROPS[backdrop];
  if (b) {
    style.sources.backdrop = { type: "raster", tiles: b.tiles, tileSize: b.tileSize, attribution: b.attribution };
    style.layers.push({ id: "backdrop", type: "raster", source: "backdrop" });
  }
  return style;
}
