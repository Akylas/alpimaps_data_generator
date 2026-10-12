// What each map mode is for, defined once.
//
// The map view and the docs tab both read this. When the two were written out separately, the
// docs described a Route mode that could not say which package it routed on, months after the
// picker was added - the same drift the CLI reference had before it was read from the binary.

export const MAP_MODES = [
  {
    id: "inspect",
    label: "Inspect",
    hint: "click a feature to read its properties",
    summary:
      "Reads the feature under the pointer straight out of the tile - layer, geometry type and \
every attribute the archive carries. This is what to check a schema change against: it shows \
what was encoded, not what the style chose to draw.",
    needs: null,
  },
  {
    id: "route",
    label: "Route",
    hint: "click waypoints, pick a costing model, route",
    summary:
      "Routes through the generated package with the same Valhalla the phone uses, embedded \
rather than served. The picker chooses which .vtiles to route on when an area has several, and \
the drawer reports the package and valhalla.json actually loaded. Sampling the elevation of a \
computed route is also how grade-aware costing gets checked.",
    needs: "a .vtiles routing package, and a build with Valhalla linked",
  },
  {
    id: "profile",
    label: "Profile",
    hint: "click a line, sample elevation from the terrain archive",
    summary:
      "Samples the terrain archive along the line you draw, densifying to 50 m and applying a \
hysteresis threshold before totalling ascent. Without that threshold, 1 m quantisation dither \
alone invents about 50 m of climb over a long route.",
    needs: "a terrain archive for the area",
  },
  {
    id: "tiles",
    label: "Tiles",
    hint: "click to dump that tile's contents as JSON",
    summary:
      "Decodes the clicked tile - MVT or MLT - and shows its layers and features as JSON, with \
the tile's z/x/y and its size on disk. Answers 'is this actually in the archive' without \
leaving the app.",
    needs: null,
  },
  {
    id: "style",
    label: "Style",
    hint: "point a MapLibre style at an archive and edit it live",
    summary:
      "Renders an archive through a real MapLibre style, re-applied as you edit it. Whatever the \
style names its sources, they are repointed at the local server, so a style written for a hosted \
tileset renders the file on disk without editing its URLs. Leaving the mode puts the real style \
back.",
    needs: null,
  },
];

/** How a terrain archive can be drawn, and why you would pick each. */
export const TERRAIN_MODE_HELP = {
  hillshade: "shaded relief - the readable view, and what the app draws",
  raster: "the encoded bytes as colour, so quantisation banding and the seam between two \
sources show as themselves rather than as shading",
  terrain3d: "drapes the DEM and tilts the camera, which is how tile edges give themselves \
away: a mismatched edge becomes a cliff",
};
