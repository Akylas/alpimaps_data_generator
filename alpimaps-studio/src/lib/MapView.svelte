<script>
  import { invoke } from "./api.js";
  import { onMount, onDestroy, tick } from "svelte";
  import { Map as MapLibreMap, NavigationControl, ScaleControl } from "maplibre-gl";
  import Compare from "@maplibre/maplibre-gl-compare";
  import "maplibre-gl/dist/maplibre-gl.css";
  import "@maplibre/maplibre-gl-compare/dist/maplibre-gl-compare.css";
  import {
    makeSource, addSourceToMap, removeSourceFromMap, applyVisibility, applyOpacity,
    applyOrder, boundsOf, baseStyle, BACKDROPS, setAllLayers,
  } from "./sources.js";
  import { tileGrid, pointToTile, fetchTileJson } from "./tiledebug.js";
  import SourcePanel from "./SourcePanel.svelte";
  import StyleEditor from "./StyleEditor.svelte";
  import Profile from "./Profile.svelte";
  import { COSTING_MODELS, readTrip, tripToGeoJson, formatDuration } from "./valhalla.js";

  let base = $state("");
  let areas = $state([]);
  let areaName = $state("");
  let backdrop = $state("");
  let error = $state("");
  let geomFilter = $state("all");
  let comparing = $state(false);
  let leftOpen = $state(true);
  let rightOpen = $state(false);
  let showTileGrid = $state(false);
  let styleApplied = $state(false);

  let mainSources = $state([]);
  let secondarySources = $state([]);

  let mode = $state("inspect");
  let drawn = $state([]);
  let profile = $state(null);
  let profileBusy = $state(false);
  let inspected = $state(null);
  let tileDump = $state(null);

  let valhallaBuilt = $state(false);
  let costing = $state("pedestrian");
  let trip = $state(null);
  let routeBusy = $state(false);

  let containerEl, mainEl, secondaryEl, shellEl;
  let mainMap = null, secondaryMap = null, compare = null, resizeObserver = null;

  let area = $derived(areas.find((a) => a.name === areaName));
  let artifacts = $derived(area?.artifacts ?? []);
  let renderable = $derived(artifacts.filter((a) => !a.probe_error && a.kind !== "valhalla_package"));

  function canonicalFirst(list) {
    return [...list].sort((a, b) => (a.variant ? 1 : 0) - (b.variant ? 1 : 0));
  }
  let terrainArt = $derived(canonicalFirst(renderable.filter((a) => a.kind === "terrain_rgb"))[0]);

  // Availability follows the data, not just the build. A build with Valhalla linked still cannot
  // route an area that has no routing package, and offering the mode anyway only produces an
  // error after the user has placed two waypoints.
  let hasRoutingTiles = $derived(artifacts.some((a) => a.kind === "valhalla_package"));
  let canRoute = $derived(valhallaBuilt && hasRoutingTiles);
  let canProfile = $derived(!!terrainArt);

  let addableMain = $derived(renderable.filter((a) => !mainSources.some((s) => s.file === a.file_name)));
  let addableSecondary = $derived(renderable.filter((a) => !secondarySources.some((s) => s.file === a.file_name)));

  function waitForBox(el) {
    return new Promise((resolve) => {
      if (el?.clientWidth && el?.clientHeight) return resolve();
      const ro = new ResizeObserver(() => {
        if (el.clientWidth && el.clientHeight) { ro.disconnect(); resolve(); }
      });
      ro.observe(el);
    });
  }

  function whenStyleReady(map, fn) {
    if (!map) return;
    if (map.isStyleLoaded()) { fn(); return; }
    let done = false;
    const attempt = () => {
      if (done || !map.isStyleLoaded()) return;
      done = true;
      clearInterval(timer);
      map.off("styledata", attempt);
      fn();
    };
    const timer = setInterval(() => { attempt(); if (done) clearInterval(timer); }, 120);
    map.on("styledata", attempt);
    setTimeout(() => { done = true; clearInterval(timer); map.off("styledata", attempt); }, 15000);
  }

  onMount(async () => {
    try {
      base = await invoke("start_tiles");
      areas = await invoke("list_areas");
      areaName = areas[0]?.name ?? "";
      valhallaBuilt = (await invoke("routing_status"))?.available ?? false;
    } catch (err) {
      error = String(err);
      return;
    }

    await waitForBox(containerEl);
    mainMap = newMap(mainEl);
    secondaryMap = newMap(secondaryEl);
    mainMap.on("click", onMapClick);
    mainMap.on("moveend", refreshTileGrid);

    // The map has to follow the window, not a fixed height: the shell is flex and the panels
    // collapse, so the container changes size without the window ever being resized.
    resizeObserver = new ResizeObserver(() => { mainMap?.resize(); secondaryMap?.resize(); });
    resizeObserver.observe(containerEl);
    if (import.meta.env.DEV) { window.__main = mainMap; window.__secondary = secondaryMap; }

    await addDefaults();
  });

  onDestroy(() => {
    resizeObserver?.disconnect();
    compare?.remove();
    mainMap?.remove();
    secondaryMap?.remove();
  });

  function newMap(container) {
    const map = new MapLibreMap({
      container, style: baseStyle(backdrop), center: [5.7, 45.4], zoom: 8,
      attributionControl: { compact: true }, maxPitch: 85,
    });
    map.addControl(new NavigationControl({ visualizePitch: true }), "top-right");
    map.addControl(new ScaleControl({ unit: "metric" }), "bottom-left");
    return map;
  }

  /**
   * Release 3D terrain before replacing a style.
   *
   * `setStyle` removes every source, including the DEM the terrain points at. Left attached, the
   * new style never finishes loading and the map wedges silently - no error, no tiles, and
   * every later call fails with "Style is not done loading".
   */
  function releaseTerrain(map) {
    if (!map) return;
    // deliberately not gated on isStyleLoaded: the moment this matters most is mid-load, which
    // is exactly when that check would skip it and leave the map to wedge
    try {
      map.setTerrain(null);
    } catch {}
  }

  /**
   * Resize both maps after a layout change.
   *
   * The ResizeObserver covers window resizes, but its callbacks are delivered as part of the
   * frame lifecycle - so a collapse that happens while the window is hidden or occluded leaves
   * the canvas at its old size until something else forces a frame. Calling it directly makes
   * the panel toggles deterministic.
   */
  async function relayout() {
    await tick();
    mainMap?.resize();
    secondaryMap?.resize();
  }

  const mapFor = (w) => (w === "secondary" ? secondaryMap : mainMap);
  const listFor = (w) => (w === "secondary" ? secondarySources : mainSources);
  const which = (source) => (mainSources.includes(source) ? "main" : "secondary");

  async function addDefaults() {
    const basemap = canonicalFirst(renderable.filter((a) => a.kind === "basemap"))[0];
    if (terrainArt) await addArtifact(terrainArt, "main");
    if (basemap) await addArtifact(basemap, "main");
  }

  async function addArtifact(artifact, w = "main") {
    const map = mapFor(w);
    if (!map || listFor(w).some((s) => s.file === artifact.file_name)) return;
    try {
      const res = await fetch(`${base}/tilejson/${artifact.area}/${artifact.file_name}`);
      if (!res.ok) throw new Error(`tilejson ${res.status}`);
      const source = makeSource(artifact, await res.json(), base);
      const apply = () => {
        addSourceToMap(map, source);
        const current = listFor(w);
        const wasEmpty = current.length === 0;
        if (w === "secondary") secondarySources = [source, ...current];
        else mainSources = [source, ...current];
        restack(w);
        if (wasEmpty) fit(source, map);
      };
      map.isStyleLoaded() ? apply() : whenStyleReady(map, apply);
    } catch (err) {
      error = String(err);
    }
  }

  function restack(w) {
    const map = mapFor(w);
    const list = listFor(w);
    applyOrder(map, list);
    for (const s of list) { applyVisibility(map, s, geomFilter); applyOpacity(map, s); }
    if (drawn.length) drawOverlay();
    if (trip) drawRoute();
    if (showTileGrid) refreshTileGrid();
  }

  function bump() {
    mainSources = [...mainSources];
    secondarySources = [...secondarySources];
  }

  function toggleSource(source) {
    source.visible = !source.visible;
    applyVisibility(mapFor(which(source)), source, geomFilter);
    bump();
  }
  function toggleLayer(source, layer) {
    layer.visible = !layer.visible;
    applyVisibility(mapFor(which(source)), source, geomFilter);
    bump();
  }
  function setAll(source, visible) {
    setAllLayers(source, visible);
    applyVisibility(mapFor(which(source)), source, geomFilter);
    bump();
  }
  function setOpacity(source, value) {
    source.opacity = value;
    applyOpacity(mapFor(which(source)), source);
    bump();
  }
  function setTerrainMode(source, mode) {
    const w = which(source);
    const map = mapFor(w);
    const was = source.terrainMode;
    source.terrainMode = mode;
    // the source type itself changes between modes, so it is rebuilt rather than re-styled
    removeSourceFromMap(map, source);
    addSourceToMap(map, source);
    // 3D is for looking at tile edges, and a flat camera shows none of them: tilt on the way in
    // and flatten on the way out, but never fight a pitch the user set themselves
    if (mode === "terrain3d" && map?.getPitch() === 0) map.easeTo({ pitch: 62, duration: 400 });
    else if (was === "terrain3d" && mode !== "terrain3d") map?.easeTo({ pitch: 0, duration: 400 });
    restack(w);
    bump();
  }

  function move(list, index, delta) {
    const target = index + delta;
    if (target < 0 || target >= list.length) return list;
    const next = [...list];
    [next[index], next[target]] = [next[target], next[index]];
    return next;
  }
  function moveIn(w, index, delta) {
    if (w === "secondary") secondarySources = move(secondarySources, index, delta);
    else mainSources = move(mainSources, index, delta);
    restack(w);
  }
  function remove(source) {
    const w = which(source);
    removeSourceFromMap(mapFor(w), source);
    if (w === "secondary") secondarySources = secondarySources.filter((s) => s !== source);
    else mainSources = mainSources.filter((s) => s !== source);
  }
  function fit(source, map = mapFor(which(source))) {
    const b = boundsOf(source);
    if (b && map) map.fitBounds(b, { padding: 20, animate: false });
  }

  $effect(() => {
    geomFilter;
    for (const w of ["main", "secondary"]) {
      const map = mapFor(w);
      if (map) for (const s of listFor(w)) applyVisibility(map, s, geomFilter);
    }
  });

  async function setBackdrop(value) {
    backdrop = value;
    styleApplied = false;
    for (const w of ["main", "secondary"]) {
      const map = mapFor(w);
      if (!map) continue;
      const list = listFor(w);
      releaseTerrain(map);
      map.setStyle(baseStyle(backdrop));
      whenStyleReady(map, () => {
        for (const s of [...list].reverse()) addSourceToMap(map, s);
        restack(w);
      });
    }
  }

  async function toggleCompare() {
    comparing = !comparing;
    if (!comparing) {
      compare?.remove();
      compare = null;
      mainMap?.resize();
      secondaryMap?.resize();
      return;
    }
    await tick();
    compare = new Compare(mainMap, secondaryMap, containerEl, {});
    compare.setSlider((containerEl?.clientWidth || 800) / 2);
    guardSwiperDrag();
  }

  function guardSwiperDrag() {
    const swiper = containerEl?.querySelector(".compare-swiper-vertical, .compare-swiper-horizontal");
    if (!swiper) return;
    swiper.addEventListener("mousedown", (e) => {
      e.preventDefault();
      document.body.classList.add("swiping");
      const stop = () => {
        document.body.classList.remove("swiping");
        document.removeEventListener("mouseup", stop);
      };
      document.addEventListener("mouseup", stop);
    });
  }

  // ------------------------------------------------------------------ tile grid

  function refreshTileGrid() {
    if (!mainMap?.isStyleLoaded()) return;
    const data = showTileGrid
      ? tileGrid(mainMap)
      : { type: "FeatureCollection", features: [] };
    const existing = mainMap.getSource("tilegrid");
    if (existing) { existing.setData(data); return; }
    if (!showTileGrid) return;
    try {
      mainMap.addSource("tilegrid", { type: "geojson", data });
      mainMap.addLayer({
        id: "tilegrid-line", type: "line", source: "tilegrid",
        paint: { "line-color": "#ff6b6b", "line-width": 1, "line-opacity": 0.8 },
      });
      mainMap.addLayer({
        id: "tilegrid-label", type: "symbol", source: "tilegrid",
        filter: ["==", "$type", "Point"],
        layout: { "text-field": ["get", "label"], "text-size": 11, "text-allow-overlap": true },
        paint: { "text-color": "#ff6b6b", "text-halo-color": "#12151a", "text-halo-width": 1.5 },
      });
    } catch {}
  }

  $effect(() => { showTileGrid; refreshTileGrid(); });

  // ------------------------------------------------------------------ interaction

  function onMapClick(e) {
    if (mode === "route") {
      drawn = [...drawn, [e.lngLat.lng, e.lngLat.lat]];
      drawOverlay();
      if (drawn.length >= 2) computeRoute();
      return;
    }
    if (mode === "profile") {
      drawn = [...drawn, [e.lngLat.lng, e.lngLat.lat]];
      drawOverlay();
      return;
    }
    if (mode === "tiles") {
      dumpTile(e.lngLat);
      return;
    }
    const features = mainMap.queryRenderedFeatures(e.point);
    if (!features.length) { inspected = null; return; }
    inspected = {
      at: [e.lngLat.lng.toFixed(5), e.lngLat.lat.toFixed(5)],
      features: features.slice(0, 8).map((f) => ({
        layer: f.sourceLayer ?? f.layer.id,
        type: f.geometry?.type,
        props: Object.entries(f.properties ?? {}),
      })),
    };
  }

  async function dumpTile(lngLat) {
    const source = mainSources.find((s) => s.vector);
    if (!source) { error = "no vector layer to read a tile from"; return; }
    const tile = pointToTile(lngLat.lng, lngLat.lat, Math.floor(mainMap.getZoom()));
    try {
      tileDump = await fetchTileJson(base, source.area, source.file, tile, source.tileEncoding);
      tileDump.file = source.file;
    } catch (err) {
      error = String(err);
    }
  }

  function drawOverlay() {
    if (!mainMap) return;
    if (!mainMap.isStyleLoaded()) { whenStyleReady(mainMap, drawOverlay); return; }
    const data = {
      type: "FeatureCollection",
      features: [
        ...(drawn.length > 1
          ? [{ type: "Feature", geometry: { type: "LineString", coordinates: drawn }, properties: {} }]
          : []),
        ...drawn.map((c) => ({ type: "Feature", geometry: { type: "Point", coordinates: c }, properties: {} })),
      ],
    };
    const existing = mainMap.getSource("drawn");
    if (existing) { existing.setData(data); return; }
    try {
      mainMap.addSource("drawn", { type: "geojson", data });
      mainMap.addLayer({ id: "drawn-line", type: "line", source: "drawn",
        paint: { "line-color": "#ffd166", "line-width": 2.5 } });
      mainMap.addLayer({ id: "drawn-pt", type: "circle", source: "drawn",
        filter: ["==", "$type", "Point"],
        paint: { "circle-color": "#ffd166", "circle-radius": 4, "circle-stroke-width": 1,
                 "circle-stroke-color": "#12151a" } });
    } catch {}
  }

  async function computeRoute() {
    if (drawn.length < 2 || !canRoute) return;
    routeBusy = true; error = "";
    try {
      const raw = await invoke("valhalla_route", { req: { area: areaName, locations: drawn, costing } });
      trip = readTrip(JSON.parse(raw).trip);
    } catch (err) {
      error = String(err);
      trip = null;
    } finally {
      routeBusy = false;
    }
    drawRoute();
  }

  function drawRoute() {
    if (!mainMap || !trip) return;
    if (!mainMap.isStyleLoaded()) { whenStyleReady(mainMap, drawRoute); return; }
    const { line, maneuvers } = tripToGeoJson(trip);
    for (const [id, data] of [["route", line], ["route-maneuvers", maneuvers]]) {
      const existing = mainMap.getSource(id);
      if (existing) { existing.setData(data); continue; }
      try { mainMap.addSource(id, { type: "geojson", data }); } catch { continue; }
      if (id === "route") {
        mainMap.addLayer({ id: "route-casing", type: "line", source: id,
          layout: { "line-cap": "round", "line-join": "round" },
          paint: { "line-color": "#0b1a33", "line-width": 8, "line-opacity": 0.65 } });
        mainMap.addLayer({ id: "route-line", type: "line", source: id,
          layout: { "line-cap": "round", "line-join": "round" },
          paint: { "line-color": "#4aa3ff", "line-width": 3.5 } });
      } else {
        mainMap.addLayer({ id: "route-maneuver-dots", type: "circle", source: id,
          paint: { "circle-color": "#ffd166", "circle-radius": 3.5,
                   "circle-stroke-width": 1, "circle-stroke-color": "#0b1a33" } });
      }
    }
  }

  async function computeProfile(line = drawn) {
    if (line.length < 2 || !terrainArt) return;
    profileBusy = true; error = "";
    try {
      profile = await invoke("elevation_profile", {
        req: { path: terrainArt.path, line, densifyM: 50, thresholdM: 3 },
      });
    } catch (err) {
      error = String(err);
    } finally {
      profileBusy = false;
    }
  }

  function clearDrawing() {
    drawn = []; trip = null; profile = null;
    drawOverlay();
    for (const id of ["route", "route-maneuvers"]) {
      if (mainMap?.getSource(id)) mainMap.getSource(id).setData({ type: "FeatureCollection", features: [] });
    }
  }

  // ------------------------------------------------------------------ style testing

  function applyCustomStyle(style, source) {
    if (!mainMap) return;
    const url = `${base}/tilejson/${source.area}/${source.file}`;
    const next = structuredClone(style);
    next.sources = next.sources ?? {};
    // repoint every vector source at the local archive, whatever the style called them
    const names = Object.keys(next.sources).filter((k) => next.sources[k]?.type === "vector");
    const targets = names.length ? names : ["SOURCE"];
    for (const name of targets) {
      next.sources[name] = { type: "vector", url, encoding: source.tileEncoding };
    }
    for (const layer of next.layers ?? []) {
      if (layer.source && layer.source !== "SOURCE" && !next.sources[layer.source]) {
        layer.source = targets[0];
      } else if (layer.source === "SOURCE") {
        layer.source = targets[0];
      }
    }
    // a custom style carries no DEM, so 3D cannot survive it; drop the mode rather than leave
    // a button claiming to be on
    for (const source of mainSources) {
      if (source.terrainMode === "terrain3d") source.terrainMode = "hillshade";
    }
    bump();
    releaseTerrain(mainMap);
    mainMap.setStyle(next);
    styleApplied = true;
    whenStyleReady(mainMap, () => { if (showTileGrid) refreshTileGrid(); });
  }

  function clearCustomStyle() {
    styleApplied = false;
    setBackdrop(backdrop);
  }

  const MODES = [
    ["inspect", "Inspect", () => true],
    ["route", "Route", () => canRoute],
    ["profile", "Profile", () => canProfile],
    ["tiles", "Tiles", () => true],
    ["style", "Style", () => true],
  ];
</script>

<div class="shell" bind:this={shellEl}>
  <div class="toolbar">
    <select value={areaName} onchange={(e) => (areaName = e.target.value)} title="area">
      {#each areas as a}<option value={a.name}>{a.name}</option>{/each}
    </select>

    <div class="tabs" role="group" aria-label="mode">
      {#each MODES as [id, label, enabled]}
        <button class:on={mode === id} disabled={!enabled()}
                title={enabled() ? "" : id === "route"
                  ? (valhallaBuilt ? "this area has no routing package" : "this build has no Valhalla linked")
                  : "this area has no terrain archive"}
                onclick={() => { mode = id; relayout(); }}>{label}</button>
      {/each}
    </div>

    <div class="seg" role="group" aria-label="geometry filter">
      {#each [["all", "All"], ["polygons", "Poly"], ["lines", "Lines"], ["points", "Pts"]] as [id, label]}
        <button class:on={geomFilter === id} onclick={() => (geomFilter = id)}>{label}</button>
      {/each}
    </div>

    <select value={backdrop} onchange={(e) => setBackdrop(e.target.value)} title="backdrop">
      <option value="">no backdrop</option>
      {#each Object.entries(BACKDROPS) as [key, b]}<option value={key}>{b.label}</option>{/each}
    </select>

    <button class:on={showTileGrid} onclick={() => (showTileGrid = !showTileGrid)} title="tile boundaries">
      Grid
    </button>

    <div class="spacer"></div>
    <button class:on={comparing} onclick={toggleCompare} disabled={!secondarySources.length && !comparing}
            title={secondarySources.length || comparing
              ? "swipe between the two maps"
              : "open the right panel and add layers to the comparison map first"}>
      Compare
    </button>
  </div>

  {#if error}<p class="warn bar-error">{error} <button class="link" onclick={() => (error = "")}>dismiss</button></p>{/if}

  <div class="body">
    <div class="side left" class:closed={!leftOpen}>
      {#if leftOpen}
        <SourcePanel sources={mainSources} title={comparing ? "Layers · right map" : "Layers"}
                     addable={addableMain} onAdd={(a) => addArtifact(a, "main")}
                     onToggleSource={toggleSource} onToggleLayer={toggleLayer}
                     onSetAllLayers={setAll} onOpacity={setOpacity} onTerrainMode={setTerrainMode}
                     onMove={(i, d) => moveIn("main", i, d)} onRemove={remove} onFit={fit} />
      {/if}
    </div>
    <button class="grip left" onclick={() => { leftOpen = !leftOpen; relayout(); }}
            title={leftOpen ? "collapse" : "layers"}>{leftOpen ? "‹" : "›"}</button>

    <div class="maps" bind:this={containerEl} class:comparing>
      <div class="map" bind:this={secondaryEl}></div>
      <div class="map" bind:this={mainEl}></div>
    </div>

    <button class="grip right" class:hot={comparing || secondarySources.length}
            onclick={() => { rightOpen = !rightOpen; relayout(); }}
            title={rightOpen ? "collapse" : "layers for the comparison map"}>{rightOpen ? "›" : "‹"}</button>
    <div class="side right" class:closed={!rightOpen}>
      {#if rightOpen}
        <SourcePanel sources={secondarySources} title="Layers · left map"
                     emptyHint="add layers here, then hit Compare to swipe between the two maps"
                     addable={addableSecondary} onAdd={(a) => addArtifact(a, "secondary")}
                     onToggleSource={toggleSource} onToggleLayer={toggleLayer}
                     onSetAllLayers={setAll} onOpacity={setOpacity} onTerrainMode={setTerrainMode}
                     onMove={(i, d) => moveIn("secondary", i, d)} onRemove={remove} onFit={fit} />
      {/if}
    </div>
  </div>

  <div class="drawer" class:tall={mode === "style"}>
    {#if mode === "style"}
      <StyleEditor sources={mainSources} {base} applied={styleApplied}
                   onApply={applyCustomStyle} onClear={clearCustomStyle} />
    {:else if mode === "route"}
      <div class="row">
        <select bind:value={costing} onchange={() => drawn.length >= 2 && computeRoute()}>
          {#each COSTING_MODELS as c}<option value={c}>{c}</option>{/each}
        </select>
        <span class="muted">{drawn.length} waypoints</span>
        <button onclick={computeRoute} disabled={drawn.length < 2 || routeBusy}>
          {routeBusy ? "Routing…" : "Route"}
        </button>
        <button onclick={() => computeProfile(trip?.points ?? drawn)}
                disabled={!trip || !canProfile || profileBusy}>
          {profileBusy ? "Sampling…" : "Elevation profile"}
        </button>
        <button class="ghost" onclick={clearDrawing} disabled={!drawn.length}>Clear</button>
      </div>
      {#if trip}
        <p class="summary">
          <strong>{trip.lengthKm.toFixed(1)} km</strong> · {formatDuration(trip.timeS)}
          · <span class="muted">{trip.maneuvers.length} maneuvers</span>
        </p>
        <Profile {profile} />
        <ol class="maneuvers">
          {#each trip.maneuvers as m}
            <li><span class="km">{m.lengthKm.toFixed(2)} km</span> {m.instruction}</li>
          {/each}
        </ol>
      {:else}
        <p class="muted hint">Click two or more points to route between them.</p>
      {/if}
    {:else if mode === "profile"}
      <div class="row">
        <span class="muted">{drawn.length} points</span>
        <button onclick={() => computeProfile()} disabled={drawn.length < 2 || profileBusy}>
          {profileBusy ? "Sampling…" : "Elevation profile"}
        </button>
        <button class="ghost" onclick={clearDrawing} disabled={!drawn.length}>Clear</button>
      </div>
      <Profile {profile} />
      {#if !profile}<p class="muted hint">Click points along a line, then sample.</p>{/if}
    {:else if mode === "tiles"}
      {#if tileDump}
        <p class="summary">
          <code>{tileDump.file}</code> {tileDump.z}/{tileDump.x}/{tileDump.y}
          {#if tileDump.empty}<span class="muted">— no tile here</span>
          {:else}<span class="muted">— {(tileDump.bytes / 1024).toFixed(1)} KB</span>{/if}
        </p>
        {#if tileDump.layers}
          <pre>{JSON.stringify(tileDump.layers, null, 1)}</pre>
        {/if}
      {:else}
        <p class="muted hint">Click the map to read that tile's contents as JSON.</p>
      {/if}
    {:else if inspected}
      <p class="muted">{inspected.at[1]}, {inspected.at[0]}</p>
      {#each inspected.features as f}
        <div class="feat">
          <h4>{f.layer} <span class="muted">{f.type}</span></h4>
          <table><tbody>
            {#each f.props as [k, v]}<tr><td>{k}</td><td>{v}</td></tr>{/each}
          </tbody></table>
        </div>
      {/each}
    {:else}
      <p class="muted hint">Click a feature to inspect it.</p>
    {/if}
  </div>
</div>

<style>
  /* the whole view is a column that fills its parent, so the map follows the window and the
     panels can collapse without anything being sized in pixels */
  .shell { display: flex; flex-direction: column; flex: 1; min-height: 0; gap: 8px;
           /* outside the flex shell (browser dev, or a tab that is not full-height) fall back */
           height: 100%; }
  .toolbar { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; flex: none; }
  .toolbar select { padding: 6px 8px; background: #12151a; border: 1px solid #303845;
                    border-radius: 5px; color: #dde3ea; font: inherit; font-size: 12px; max-width: 180px; }
  .toolbar button { background: #262d38; color: #9aa5b1; padding: 6px 11px; font-size: 12px; }
  .toolbar button.on { background: #2d5f4a; color: #fff; }
  .toolbar button:disabled { opacity: .45; cursor: not-allowed; }
  .tabs { display: flex; gap: 2px; }
  .seg { display: flex; gap: 1px; }
  .seg button { border-radius: 0; padding: 6px 9px; }
  .seg button:first-child { border-radius: 5px 0 0 5px; }
  .seg button:last-child { border-radius: 0 5px 5px 0; }
  .spacer { flex: 1; }
  .bar-error { margin: 0; flex: none; }

  .body { display: flex; gap: 0; flex: 1; min-height: 0; }
  .side { width: 250px; flex: none; overflow: auto; padding-right: 8px; }
  .side.right { padding-right: 0; padding-left: 8px; }
  .side.closed { width: 0; padding: 0; overflow: hidden; }
  .grip { flex: none; width: 14px; background: #161b22; border: 1px solid #262d38; color: #6b7684;
          cursor: pointer; padding: 0; font-size: 11px; border-radius: 4px; margin: 0 2px; }
  .grip:hover { color: #dde3ea; }
  .grip.hot { color: #7cc9a0; border-color: #2d5f4a; }

  .maps { position: relative; flex: 1; min-width: 0; border-radius: 8px; overflow: hidden;
          border: 1px solid #262d38; }
  .map { position: absolute; inset: 0; }
  .maps:not(.comparing) .map:first-child { visibility: hidden; }
  :global(body.swiping) { user-select: none; }

  .drawer { flex: none; max-height: 240px; overflow: auto; }
  .drawer.tall { max-height: 420px; }
  .row { display: flex; gap: 8px; align-items: center; margin-bottom: 8px; flex-wrap: wrap; }
  .row select { padding: 6px 8px; background: #12151a; border: 1px solid #303845;
                border-radius: 5px; color: #dde3ea; font: inherit; font-size: 12px; }
  .summary { margin: 4px 0; font-size: 14px; }
  .maneuvers { margin: 8px 0 0; padding-left: 0; list-style: none; font-size: 12px; color: #9aa5b1; }
  .maneuvers li { padding: 2px 0; border-bottom: 1px solid #1e242d; }
  .km { display: inline-block; width: 62px; text-align: right; color: #6b7684;
        font-variant-numeric: tabular-nums; margin-right: 8px; }
  .feat { border-top: 1px solid #262d38; padding-top: 8px; margin-top: 8px; }
  h4 { margin: 0 0 4px; font-size: 13px; }
  table { width: 100%; border-collapse: collapse; font-size: 12px; }
  td { padding: 2px 6px 2px 0; vertical-align: top; }
  td:first-child { color: #6b7684; width: 34%; }
  pre { margin: 0; font-size: 11px; color: #98a3b0; white-space: pre-wrap; word-break: break-all; }
  .muted { color: #6b7684; }
  .hint { font-size: 13px; }
  .warn { color: #d99a5b; }
  .link { background: none; border: 0; color: #7c8896; font: inherit; font-size: 11px;
          text-decoration: underline; cursor: pointer; padding: 0; }
</style>
