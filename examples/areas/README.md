# Sample areas

Osmosis `.poly` boundaries to clip a build to. Pass one to a step with `--polygon` (basemap, routes),
`--poly-shape` (terrain) or `--poly` (package):

```shell
cairn basemap --area rhone-alpes --polygon examples/areas/rhone-alpes.poly
```

| File | Covers |
| --- | --- |
| `rhone-alpes.poly`, `rhone-alpes.poly.geojson` | the Rhône-Alpes region, France — the area the presets were measured on |
| `europe.poly` | a rough outline of Europe |

You rarely need these: with no polygon given, a step downloads the area's own boundary from Geofabrik. Make
your own with any tool that writes `.poly`, or convert GeoJSON with
[`tools/legacy/scripts/utils/polygon2geojson.py`](../../tools/legacy/scripts/utils/polygon2geojson.py).
