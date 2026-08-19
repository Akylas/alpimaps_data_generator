// Reading Valhalla trip responses.
//
// Adapted from gps-mocker-rs (`src/lib/valhalla.ts`). Two details there are worth carrying over
// verbatim, because both fail quietly rather than loudly.

export const COSTING_MODELS = ["auto", "bicycle", "pedestrian", "motorcycle", "bus", "truck"];

/**
 * Valhalla encodes shapes as Google polyline with **six** decimals, not the usual five.
 * Decoding at five puts the route in roughly the right shape a tenth of the way from the
 * equator, which reads as a bug in the tiles rather than in the decoder.
 */
export function decodePolyline(encoded, precision = 6) {
  const factor = Math.pow(10, precision);
  const points = [];
  let index = 0;
  let lat = 0;
  let lon = 0;

  while (index < encoded.length) {
    let result = 0;
    let shift = 0;
    let byte;
    do {
      byte = encoded.charCodeAt(index++) - 63;
      result |= (byte & 0x1f) << shift;
      shift += 5;
    } while (byte >= 0x20);
    lat += result & 1 ? ~(result >> 1) : result >> 1;

    result = 0;
    shift = 0;
    do {
      byte = encoded.charCodeAt(index++) - 63;
      result |= (byte & 0x1f) << shift;
      shift += 5;
    } while (byte >= 0x20);
    lon += result & 1 ? ~(result >> 1) : result >> 1;

    points.push([lon / factor, lat / factor]);
  }
  return points;
}

/**
 * Flatten a trip into one continuous shape.
 *
 * Legs carry their own shape and leg-relative manoeuvre indices, so both need re-basing, and
 * consecutive legs repeat the point they share - kept once, or every waypoint shows a duplicate.
 */
export function readTrip(trip) {
  const points = [];
  const maneuvers = [];

  for (const leg of trip?.legs ?? []) {
    const legPoints = decodePolyline(leg.shape ?? "");
    const offset = points.length > 0 ? points.length - 1 : 0;
    points.push(...(points.length > 0 ? legPoints.slice(1) : legPoints));

    for (const m of leg.maneuvers ?? []) {
      maneuvers.push({
        pointIndex: Math.min(points.length - 1, offset + (m.begin_shape_index ?? 0)),
        type: m.type ?? 0,
        instruction: m.instruction ?? "",
        // Valhalla reports lengths in the requested units; the request asks for kilometres
        lengthKm: m.length ?? 0,
        timeS: m.time ?? 0,
      });
    }
  }

  return {
    points,
    maneuvers,
    lengthKm: trip?.summary?.length ?? 0,
    timeS: trip?.summary?.time ?? 0,
  };
}

/** GeoJSON for the route line and one point per manoeuvre. */
export function tripToGeoJson(trip) {
  return {
    line: {
      type: "FeatureCollection",
      features: trip.points.length > 1
        ? [{ type: "Feature", geometry: { type: "LineString", coordinates: trip.points }, properties: {} }]
        : [],
    },
    maneuvers: {
      type: "FeatureCollection",
      features: trip.maneuvers.map((m, i) => ({
        type: "Feature",
        geometry: { type: "Point", coordinates: trip.points[m.pointIndex] ?? trip.points[0] },
        properties: { index: i, instruction: m.instruction },
      })).filter((f) => f.geometry.coordinates),
    },
  };
}

export function formatDuration(seconds) {
  const total = Math.round(seconds / 60);
  const hours = Math.floor(total / 60);
  const minutes = total % 60;
  return hours > 0 ? `${hours} h ${minutes} min` : `${minutes} min`;
}
