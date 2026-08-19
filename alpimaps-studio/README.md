# AlpiMaps Studio

Desktop front end for the AlpiMaps tile pipeline. See [`../docs/DESKTOP_APP_PLAN.md`](../docs/DESKTOP_APP_PLAN.md)
for the full plan. Milestones **1.1–1.3** (spike, workspace, catalog), **2** (preview), **3** (generation) and
part of **4** (native ports) are in. Still outstanding in 4: the GeoTIFF raster source for
terrain, Valhalla routing preview, and packaging/notarisation.

## Layout

```
core/        studio-core: no Tauri dependency, tests in seconds
src-tauri/   thin Tauri shell: commands, event forwarding
src/         Svelte UI
```

The split is deliberate. The log parser and toolchain probes are the parts that need frequent,
fast iteration; keeping them out of the Tauri crate means `cargo test -p studio-core` runs in
under a second instead of behind a webview build.

## Run

```bash
npm install && npm run tauri dev
```

Fast inner loop, no GUI:

```bash
cargo test -p studio-core
```

End-to-end against a real planetiler build (downloads the monaco extract, ~20s):

```bash
cargo run -p studio-core --example spike -- ../planetiler/planetiler-dist/target/planetiler-dist-0.10.3-SNAPSHOT-with-deps.jar /tmp/spike
```

## Native ports

### Valhalla routing

`valhalla/routing.rs` routes in-process through `valhalla::tyr::actor_t`, reached by a small C
shim (`core/src-cpp/valhalla_shim.cc`). JSON in, JSON out, no `valhalla_service` and therefore
no prime_server and no zmq.

Linking is the interesting part. Valhalla's link line runs to about a hundred dylibs, so
`build.rs` reads the one CMake already produced - `link.txt` and `flags.make` from a built tree -
rather than restating it, which keeps it working across Homebrew upgrades. Entirely opt-in: with
no Valhalla found the crate builds exactly as before and routing reports itself unavailable, and
a test asserts each side of that.

Three things that had to be right:

* `rustc-link-arg` goes to the compiler driver, not the linker, so `-Wl,` prefixes must survive.
  Stripping them turns `-Wl,-search_paths_first` into a flag clang rejects.
* Not every target's `link.txt` is equivalent - one that never downloads tiles omits curl, one
  that never reads compressed tiles omits zlib. A known-complete target is preferred and both
  are appended.
* `actor_t` validates the **whole** config on construction, not the parts a route uses: a config
  without `service_limits.isochrone.max_contours` is refused for a plain route. So the pipeline's
  own `valhalla.json` is used as the template with only `tile_dir` overridden - and any
  `tile_extract` removed, since a stale extract would silently win over the directory asked for.

Routing runs against an **unpacked `.vtiles`**, not the intermediate tile directory, so what is
exercised is the artefact that ships. Grenoble to Chambery: 135 tiles unpacked in 1.2 s, actor
ready in 0.01 s, routed in 0.08 s, 59.2 km / 39 min over 25 maneuvers.

### Valhalla package

`core/src/valhalla/` replaces `scripts/build_valhalla_package.py`. Verified against the existing
package: all **135 of 135** tile ids resolved to the right `.gph` file on disk, and a full
rebuild produced **212.9 MiB against the reference 219.0 MiB, −2.80%**, from an identical
493.8 MiB of input.

Two carried-over details are load-bearing. **zopfli** rather than zlib, because it is ~3%
smaller and still emits ordinary gzip, so no reader changes. And **`PRAGMA page_size=4096`**,
because at SQLite's 512-byte default a ~1 MB graph tile spans thousands of overflow pages, each
spending four bytes on a next-page pointer.

One deliberate difference: the Python builds the tile path with `id /= 1000`, which is *float*
division in Python 3. It survives only because the ids in use stay small enough for a double to
hold exactly. This port uses integer division, which agrees on every id in the reference package
and does not degrade beyond it.

Full zopfli takes 136s here against the script's ~87s, so the Rust path is currently slower -
the crate's default iteration count differs, and output is 0.5 MiB larger than the Python
zopfli's 212.4 MiB. Same format, marginally different tuning.

### Terrain RGB

`core/src/terrain/` renders terrarium tiles from SRTM `.hgt` with no GDAL and no PROJ. The files
are 3601x3601 big-endian `i16` on a geographic grid, so longitude and latitude index them
directly and the only projection maths needed is the inverse web-mercator for pixel centres.

Projected raster sources are read too. `terrain/geotiff.rs` is a read-only BigTIFF reader for the
44 GB IGN raster - 198000x196000 Float32, 256-pixel tiles, ZSTD with the floating-point
predictor, plus five overview levels - and `terrain/lambert93.rs` implements EPSG:2154 directly
from the EPSG 9802 formulas. Neither GDAL nor PROJ is linked.

Checked against `gdallocationinfo` at five points, the worst disagreement is **0.298 m**, and
that residual is bilinear-versus-nearest sampling rather than decoding. Checked against the
existing terrain archive, both now reading the same `sources.json`: at z13 mean |delta| is
**0.0 m with a worst case of 1 m**, exactly one quantisation step, 100% within 5 m.

Two traps, both silent:

* **`sources.json` is lowest-priority-first** (`build_terrain_rgb.py`: *"Sources are listed
  lowest priority first"*). Reading it the other way round still returns a sensible elevation
  for every point, just from the coarser source - mean error against the reference was 4.3 m
  with the order inverted and 0.9 m with it right, at a zoom whose quantisation step is 8 m.
* **`Level` was cloned per sample.** It holds the tile index, and the full-resolution level of
  the IGN raster has 592,884 tiles, so every pixel copied ~9.5 MB. Nine z13 tiles took 296
  seconds; behind an `Arc` the same nine take 0.3.

Boundaries are feathered like the generator's. Its `composite()` builds
`weight = box_blur(valid) * valid`, so a better source fades in *inside* its own coverage - zero
at its data boundary, one `--blur` metres further in - and the `* valid` factor stops the ramp
leaking past the edge, because past the edge there is nothing to fade towards. A box blur of a
0/1 mask is the valid fraction of the neighbourhood, so the same shape comes from probing
coverage on a ring per point rather than convolving an array, which this pipeline never
materialises. The generator blurs twice for a triangular ramp; one ring is closer to linear,
differing in the middle of the fade and not at either end. Costs about 5x, and `blur_m: 0.0`
restores the hard switch.

The quantisation ramp is ported exactly, including the trap: the Python raises
`max_round_digits` to `round_digits` when it is smaller, so its default of 0 disables the ramp
entirely. A test pins that behaviour.

## Generation

Steps carry a dependency graph, so selecting only *Valhalla package* runs
`Download OSM → Elevation tiles → Valhalla tiles → Valhalla package`. Steps execute one at a
time: each planetiler run already gets its own `--tmpdir`, and serialising removes the whole
class of sort-chunk corruption rather than just the instance that was hit.

Terrain, hillshade and the valhalla steps are still shell scripts. They appear in the graph
marked *not wired* and the runner says so rather than silently doing nothing.

### Options

`core/src/steps/options.rs` is a declarative schema; the form is generated from it and the same
definitions render the command line, so the two cannot drift. Every flag name was read out of
the sources - stock ones from `PlanetilerConfig`, custom ones from the fork's `Route` and
`Landcover` layers.

**Defaults are deliberately absent.** An option carries a `hint` describing what planetiler does
when the flag is omitted, but the schema never asserts that value, and an unset option emits
nothing. The alternative - encoding a guess at planetiler's default and always emitting it -
silently changes builds whenever the guess is wrong.

### Presets

The `bench/` workflow made first-class. The measured flag sets ship as built-ins: `measured` for
the basemap (-11.7%) and for routes (-19.4%), plus `stock` as a baseline. Built-ins are merged in
at read time rather than written to disk, so they improve with the app while a user preset of
the same name still wins. A test asserts every built-in only references options the schema
actually defines - otherwise a "tuned" preset quietly becomes a stock build.

## Map view

Modes across the top: **Inspect**, **Route**, **Profile**, **Tiles**, **Style**. Route and Profile
are enabled from the *data*, not just the build - a binary with Valhalla linked still cannot route
an area with no routing package, and offering the mode anyway only fails after two waypoints have
been placed.

Layers stack per side, in order, with per-source and per-layer visibility, all/none toggles and
opacity. Both side panels collapse to give the map the width.

Terrain archives render three ways. **hillshade** is the useful view; **raster** paints the
encoded bytes directly, so quantisation banding and the seam between two sources show as
themselves rather than as shading; **3D** drapes the DEM, which is how tile edges give themselves
away - a mismatched edge is a cliff.

**Tiles** mode dumps whichever tile was clicked as JSON, decoded from MVT or MLT, so attribute
work can be checked against what is actually in the archive rather than against what the renderer
chose to draw. **Grid** overlays tile boundaries with their z/x/y.

**Style** mode renders an archive through a real MapLibre style, edited in place. Whatever the
style names its sources, they are repointed at the local server, so a style written for a hosted
tileset renders the file on disk without editing its URLs.

### Three bugs the rewrite exposed

* **3D terrain wedged every style change.** `setStyle` removes every source, including the DEM
  the terrain points at; left attached, the new style never finishes loading. No error, no tiles,
  and every later call fails with "Style is not done loading". Terrain is now released first -
  and deliberately not gated on `isStyleLoaded()`, because the moment it matters most is mid-load,
  which is exactly when that check would skip it.
* **The map did not follow the window.** The view was a fixed 520 px. It is now a flex column,
  and the panel toggles call `resize()` directly rather than waiting on the ResizeObserver, whose
  callbacks arrive with the frame lifecycle and so never arrive at all while the window is hidden.
* **`pbf` v5 has no default export.** `import Pbf from "pbf"` builds under some bundlers and
  fails under others; the reader is `PbfReader`, by name.



Layer handling is adapted from [mbview-rs](https://github.com/farfromrefug/mbview-rs)
(`src/lib/sources.ts`), which already had this right. Two ideas kept verbatim:

* **Visibility is derived, never toggled.** The source flag, the per-layer flag and the geometry
  filter are three independent inputs; anything that caches their combination drifts the moment
  one changes behind its back. `applyVisibility` recomputes from scratch.
* **Ordering by walking bottom-up and moving each layer to the top.** That leaves any backdrop
  underneath without needing to know a single one of its layer names.

Comparison uses `@maplibre/maplibre-gl-compare` rather than a hand-rolled clip-path divider - it
syncs both maps by itself, which the hand-rolled one did not. Any number of archives can be
stacked per side, reordered, and toggled per source-layer; the geometry filter switches between
all / polygons / lines / points and deliberately leaves rasters alone.

Three things that make it work, each of which fails silently otherwise:

* **`events` polyfill.** The compare plugin is CommonJS and calls `require("events")
  .EventEmitter`. Without the polyfill the constructor throws `EventEmitter is not a constructor`
  and no swiper is ever created.
* **`encoding: 'mlt'` on the source.** MLT is *not* auto-detected - the tiles arrive and fail to
  decode. The tile server now reports `tileEncoding` in its TileJSON, kept distinct from the
  DEM `encoding` field, because MapLibre calls both "encoding" on different source types.
* **maplibre-gl 5.x, and no `optimizeDeps.exclude`.** The worker 404 that produced a blank map
  earlier was specific to 6.x's multi-file `.mjs` dist; 5.x ships a single CJS bundle that
  pre-bundles cleanly, and excluding it instead breaks named-export extraction.

### Terrain archives are not always named that way

Two generations of the pipeline wrote terrain RGB. The current script stamps `encoding`; the
older rio-rgbify path stamped only `round-digits` (the per-zoom quantisation ramp, e.g.
`" 3 4 5 6 7 7 7 7"`) and no encoding at all - and those files are still called `_hillshade`.
They decode as **mapbox**, not terrarium. Classifying on the name would render elevation data as
flat imagery, so the catalog infers from `round-digits` and defaults those to mapbox.

## Why the map was blank

Four separate faults, three of them real. Recorded because each fails *silently* - no console
error, no exception, just an empty canvas.

1. **`glyphs: undefined`.** The empty-style template spread a `glyphs` key set to `undefined`.
   MapLibre's validator sees the key present and rejects the whole style: *"glyphs: string
   expected, undefined found"*. Omitting the key is not the same as setting it to undefined.
2. **Vite's dep optimiser broke the worker.** It rewrote `maplibre-gl` into `.vite/deps/` but did
   not carry `maplibre-gl-worker.mjs` with it, so the worker URL 404'd. No worker means no tile
   is ever parsed, and `load` simply never fires. Fixed with `optimizeDeps.exclude`. This would
   have broken `npm run tauri dev` too, since that serves through the same Vite dev server.
3. **Zero-size container.** MapLibre built on a 0x0 element never initialises - it stays at its
   400x300 fallback canvas, fires no `load`, requests no tiles, and reports no error. The map is
   now created only after the container has a box, and a `ResizeObserver` keeps it in step.
4. **Overlapping `setStyle`.** `refreshStyles` ran from both `onMount` and an effect; the earlier
   async run resolved last and re-applied a stale style, leaving the map with no sources
   (*"Style is not done loading, rebuilding from scratch"*). Runs are now serialised by token.

The fourth red herring: in a *backgrounded* browser tab `requestAnimationFrame` never fires, so
MapLibre never renders and the style never finishes loading. That one is a test-harness artifact,
not an app bug - but it looks identical to a real failure from the outside.

`src/lib/api.js` exists because of this: it routes backend calls to Tauri when running inside the
webview, and to the standalone tile server otherwise, so the whole UI can be driven from an
ordinary browser. Start it with
`cargo run -p studio-core --example serve -- ../alpimaps_mbtiles --hold` (port 8787) and
`npm run dev`.

## Command line

`alpimaps` runs the same code the app runs, so a build started from a terminal and one started
from the GUI produce the same bytes. It replaces the shell scripts it mirrors and keeps their
shape.

```
alpimaps catalog --stats
alpimaps basemap --area rhone-alpes --preset measured -o simplify_tolerance=0.6
alpimaps terrain --area rhone-alpes --maxzoom 13 --blur 1000
alpimaps package --area rhone-alpes --compression zopfli
alpimaps route  --tiles .../rhone-alpes.vtiles --point 5.72,45.19 --point 5.92,45.56
alpimaps profile --terrain .../rhone-alpes_terrain.mbtiles --point 6.5,45.35 --point 6.9,45.42
alpimaps serve  --tiles .../rhone-alpes.vtiles
alpimaps options basemap --presets
```

`-o key=value` overrides are checked against the same option schema the GUI form is generated
from, so an unknown key is refused rather than forwarded. Planetiler ignores flags it does not
recognise, which would otherwise turn a typo into a build that quietly used the default.
`--dry-run` prints the exact command instead of running it.

Each binary crate carries its own `build.rs` including `valhalla-link.rs`. Cargo link directives
do not propagate: `cargo:rustc-link-arg` applies only to the crate that emits it, so a binary
linking studio-core has to repeat them or the link fails on undefined symbols.

## Preview

`cargo run -p studio-core --example serve -- ../alpimaps_mbtiles` serves every renderable
archive; `--hold` keeps it up so a browser can point at it.

Three details decide whether a tile renders or silently blanks:

* **Row order.** mbtiles stores `tile_row` in TMS (origin bottom-left); XYZ counts from the top.
  `tms_row = (1 << z) - 1 - y`.
* **Content-Encoding.** Planetiler gzips MVT *and* MLT blobs, but the terrain WebP tiles are
  stored raw (`RIFF`, not `1f 8b`). The `compression` metadata key is not consistent across
  producers, so the gzip magic is sniffed per blob. Verified against the real archives: vector
  serves `enc=gzip`, raster serves none.
* **Content-Type.** MLT needs `application/vnd.maplibre-vector-tile`. MapLibre GL JS 6.4.1
  decodes MLT natively from an ordinary `vector` source — no plugin, no opt-in.

Terrain RGB becomes a `raster-dem` source with `encoding` carried through from the archive's own
metadata, so MapLibre decodes terrarium without any extra work. There is no cartographic style:
layers are generated from the TileJSON's `vector_layers`, one fill/line/circle per source layer,
which is the point — the map shows what is *in* the tiles.

## Elevation

`cargo run -p studio-core --example elevation -- <terrain.mbtiles>` samples known points and
draws a profile.

Validated against the real terrain archive: valley points land within 2 m (Grenoble −2 m, Lac du
Bourget exact). Sharp summits read 25–35 m low, which is what a 512-pixel z13 grid (~9.5 m/px at
this latitude) does to a knife-edge peak, not a defect.

**Ascent totals use hysteresis, and must.** The terrain is quantised to 1 m, so a near-flat
traverse dithers between two adjacent levels; summing raw deltas turns that dither into phantom
climb. A synthetic 1 m dither over 100 points yields **50 m** of invented ascent at zero
threshold and **0 m** at the 3 m default. On a real 32 km Vanoise traverse the difference is
+34 m. Both numbers are covered by tests.

## Output catalog

`cargo run -p studio-core --example catalog -- ../alpimaps_mbtiles --stats`

**Classification comes from the `metadata` table, not the filename.** `format` alone separates
MVT (`pbf`) from MLT (`application/vnd.maplibre-vector-tile`) from raster (`webp`) from routing
(`gph3`), and an `encoding` key is what distinguishes terrain RGB from a plain hillshade — both
are webp. So `rhone-alpes_mlt.mbtiles` is correctly a *basemap* with variant `mlt`, which no
filename rule would get right. Filenames only supply the variant, and act as fallback when a
file will not open.

**Unrecognised variants are kept, not hidden.** `.vtiles.base`, `_hillshade.mbtiles.old`,
`_mlt.mbtiles` are all present in a working tree and are exactly the files a comparison wants.
Dotfiles (`.DS_Store`) and files belonging to another area are skipped.

**Addressed bytes are not bytes on disk.** Every planetiler build here uses `--compact-db`,
where `tiles` is a view over `tiles_shallow` joined to `tiles_data`, and one blob can serve many
(z,x,y). Summing the view therefore overstates the file. The catalog reports both, and the gap
is the deduplication saving. Measured on the current output that gap is *0.3%* on the basemap
and 0% on everything else — `--compact-db` is buying almost nothing at these sizes.

## What the spike establishes

**JRE resolution.** `toolchain::find` tries, in order: settings override, app-managed download,
`$JAVA_HOME`, `$PATH`. Anything below Java 21 is rejected. `toolchain::download` pulls a JRE
from Adoptium for the host platform and extracts it, so a machine with no Java still works.

**Planetiler as a subprocess.** Never embedded — see the module docs in
`core/src/steps/planetiler.rs` for the four reasons (heap, single-use instance, mmap cleanup on
cancel, classpath). Cancellation kills the process rather than signalling it.

**Per-run tmpdir.** Every job passes its own `--tmpdir`. Two planetiler processes sharing
`data/tmp` delete each other's sort chunks; a GUI makes that mistake one click away.

## What the log parser learned the hard way

Progress has to be scraped from stdout. Four things the format does that a naive parser gets
wrong, each covered by a test:

1. **`read 1x(19% 0.2s)` is not progress** — it is per-worker CPU time, and it is the most
   common percent-bearing line in the log. Progress percentages live inside `[...]`; summary
   percentages live inside `(...)`. Brackets are the discriminator, not "first percent found".
2. **Continuation lines carry `gc: 0%`** and have no `time LEVEL [stage] -` prefix. Requiring
   the prefix drops them.
3. **The run end is not the line without a stage.** It is logged under `[archive]`, two lines
   below that stage's own `Finished in`. `FINISHED!` is the only reliable terminator, and it
   carries no duration — the total has to be remembered from the preceding `Finished in`.
4. **`--loginterval` defaults to `10s`.** A 54-second rhone-alpes build emits only *six*
   progress lines at that setting. Every job overrides it to `1s`; the same monaco build then
   produced 310. Without this the bar looks frozen.

`Finished z12 in 1s ..., now starting z13` is also parsed. The archive stage is the bulk of a
build, so per-zoom completions are the only fine-grained signal inside it.

The parser never fails a line — anything unrecognised becomes `Log`. The format is not a
contract and upstream merges move it.

## Tests

`core/tests/real_log.rs` runs the parser over a real captured build log
(`../bench/base.log`) and is self-calibrating: it counts percent-bearing bracketed lines
independently and requires the parser to have found exactly those, so it catches both misses
and false positives without hard-coding a number.
