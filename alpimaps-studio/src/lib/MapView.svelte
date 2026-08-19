<script>
  import { invoke } from "./api.js";
  import { onMount, onDestroy, tick } from "svelte";
  import { Map as MapLibreMap, NavigationControl, ScaleControl } from "maplibre-gl";
  import Compare from "@maplibre/maplibre-gl-compare";
  import "maplibre-gl/dist/maplibre-gl.css";
  import "@maplibre/maplibre-gl-compare/dist/maplibre-gl-compare.css";
  import {
    makeSource, addSourceToMap, removeSourceFromMap, applyVisibility, applyOpacity,
    applyOrder, boundsOf, baseStyle, BACKDROPS,
  } from "./sources.js";
  import SourcePanel from "./SourcePanel.svelte";
  import Profile from "./Profile.svelte";
  import { COSTING_MODELS, readTrip, tripToGeoJson, formatDuration } from "./valhalla.js";

  let base = $state("");
  let areas = $state([]);
  let areaName = $state("");
  let backdrop = $state("");
  let error = $state("");
  let geomFilter = $state("all");
  let comparing = $state(false);

  // one ordered list per map; sources[0] draws on top
  let mainSources = $state([]);
  let secondarySources = $state([]);
  let addTo = $state("main");

  let mode = $state("inspect");
  let routingReady = $state(false);
  let costing = $state("pedestrian");
  let trip = $state(null);
  let routeBusy = $state(false);
  let drawn = $state([]);
  let profile = $state(null);
  let profileBusy = $state(false);
  let inspected = $state(null);

  let containerEl, mainEl, secondaryEl;
  let mainMap = null, secondaryMap = null, compare = null, resizeObserver = null;

  let area = $derived(areas.find((a) => a.name === areaName));
  let available = $derived(
    (area?.artifacts ?? []).filter((a) => !a.probe_error && a.kind !== "valhalla_package")
  );
  /// Canonical build first: `.old` and `_mlt` style variants are kept in the catalog on purpose,
  /// but picking one as the default terrain source silently uses last month's data.
  function canonicalFirst(list) {
    return [...list].sort((a, b) => (a.variant ? 1 : 0) - (b.variant ? 1 : 0));
  }
  let terrainArt = $derived(canonicalFirst(available.filter((a) => a.kind === "terrain_rgb"))[0]);

  /**
   * Run `fn` once the style will accept `addSource`.
   *
   * Deferring on `styledata` alone can strand: the callback runs, the style is still not loaded,
   * it re-registers, and if no further `styledata` arrives the work never happens. Polling as
   * well means the last event is not load-bearing. `idle` is not used because it never fires
   * while the window is hidden.
   */
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
    const timer = setInterval(() => {
      attempt();
      if (done) clearInterval(timer);
    }, 120);
    map.on("styledata", attempt);
    // give up rather than poll forever if the style never settles
    setTimeout(() => { done = true; clearInterval(timer); map.off("styledata", attempt); }, 15000);
  }

  function waitForBox(el) {
    return new Promise((resolve) => {
      if (el?.clientWidth && el?.clientHeight) return resolve();
      const ro = new ResizeObserver(() => {
        if (el.clientWidth && el.clientHeight) { ro.disconnect(); resolve(); }
      });
      ro.observe(el);
    });
  }

  onMount(async () => {
    try {
      base = await invoke("start_tiles");
      areas = await invoke("list_areas");
      areaName = areas[0]?.name ?? "";
    } catch (err) {
      error = String(err);
      return;
    }

    try {
      routingReady = (await invoke("routing_status"))?.available ?? false;
    } catch {
      routingReady = false;
    }

    // MapLibre built on a zero-size container never initialises and reports nothing
    await waitForBox(containerEl);
    mainMap = newMap(mainEl);
    secondaryMap = newMap(secondaryEl);
    mainMap.on("click", onMapClick);
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
      container,
      style: baseStyle(backdrop),
      center: [5.7, 45.4],
      zoom: 8,
      attributionControl: { compact: true },
    });
    map.addControl(new NavigationControl({ visualizePitch: true }), "top-right");
    map.addControl(new ScaleControl({ unit: "metric" }), "bottom-left");
    return map;
  }

  function mapFor(which) {
    return which === "secondary" ? secondaryMap : mainMap;
  }
  function listFor(which) {
    return which === "secondary" ? secondarySources : mainSources;
  }

  async function addDefaults() {
    const basemap = canonicalFirst(available.filter((a) => a.kind === "basemap"))[0];
    // terrain first so the basemap ends up drawn above it
    if (terrainArt) await addArtifact(terrainArt, "main");
    if (basemap) await addArtifact(basemap, "main");
  }

  async function addArtifact(artifact, which = addTo) {
    const map = mapFor(which);
    if (!map) return;
    if (listFor(which).some((s) => s.artifact.file_name === artifact.file_name)) return;
    try {
      const res = await fetch(`${base}/tilejson/${artifact.area}/${artifact.file_name}`);
      if (!res.ok) throw new Error(`tilejson ${res.status}`);
      const source = makeSource(artifact, await res.json(), base);
      // read the list *inside* apply, never capture it: two adds in flight would otherwise
      // both prepend to the same stale array and the second would drop the first
      const apply = () => {
        addSourceToMap(map, source);
        const current = listFor(which);
        const wasEmpty = current.length === 0;
        if (which === "secondary") secondarySources = [source, ...current];
        else mainSources = [source, ...current];
        restack(which);
        if (wasEmpty) fit(source, map);
      };
      // `idle` never fires while the window is hidden, so only wait on it when the style is
      // genuinely still loading
      if (map.isStyleLoaded()) apply();
      else map.once("styledata", apply);
    } catch (err) {
      error = String(err);
    }
  }

  function restack(which) {
    const map = mapFor(which);
    const list = listFor(which);
    applyOrder(map, list);
    for (const s of list) {
      applyVisibility(map, s, geomFilter);
      applyOpacity(map, s);
    }
    if (mode === "draw") drawOverlay();
  }

  function which(source) {
    return mainSources.includes(source) ? "main" : "secondary";
  }

  function toggleSource(source) {
    source.visible = !source.visible;
    applyVisibility(mapFor(which(source)), source, geomFilter);
    mainSources = [...mainSources];
    secondarySources = [...secondarySources];
  }

  function toggleLayer(source, layer) {
    layer.visible = !layer.visible;
    applyVisibility(mapFor(which(source)), source, geomFilter);
    mainSources = [...mainSources];
    secondarySources = [...secondarySources];
  }

  function setOpacity(source, value) {
    source.opacity = value;
    applyOpacity(mapFor(which(source)), source);
    mainSources = [...mainSources];
    secondarySources = [...secondarySources];
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
    // setStyle drops every layer, so each source has to be re-added afterwards
    for (const w of ["main", "secondary"]) {
      const map = mapFor(w);
      if (!map) continue;
      map.setStyle(baseStyle(backdrop));
      const list = listFor(w);
      map.once("idle", () => {
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
    // the class change has to land before the plugin measures the container, or it clips
    // against a stale layout
    await tick();
    compare = new Compare(mainMap, secondaryMap, containerEl, {});
    compare.setSlider((containerEl?.clientWidth || 800) / 2);
    guardSwiperDrag();
  }

  /**
   * The plugin binds its own `mousedown` and never calls preventDefault, so dragging the swiper
   * also starts a text selection that smears across every control the pointer crosses. This
   * runs alongside - preventDefault does not stop the plugin's own handler - and suppresses
   * selection only for the length of the drag.
   */
  function guardSwiperDrag() {
    const swiper = containerEl?.querySelector(
      ".compare-swiper-vertical, .compare-swiper-horizontal"
    );
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

  function onMapClick(e) {
    if (mode === "route") {
      drawn = [...drawn, [e.lngLat.lng, e.lngLat.lat]];
      drawOverlay();
      if (drawn.length >= 2) computeRoute();
      return;
    }
    if (mode === "draw") {
      drawn = [...drawn, [e.lngLat.lng, e.lngLat.lat]];
      drawOverlay();
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

  function drawOverlay() {
    if (!mainMap) return;
    if (!mainMap.isStyleLoaded()) {
      whenStyleReady(mainMap, drawOverlay);
      return;
    }
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
    if (drawn.length < 2 || !routingReady) return;
    routeBusy = true;
    error = "";
    try {
      const raw = await invoke("valhalla_route", {
        req: { area: areaName, locations: drawn, costing },
      });
      trip = readTrip(JSON.parse(raw).trip);
    } catch (err) {
      error = String(err);
      trip = null;
    } finally {
      routeBusy = false;
    }
    // drawing is separate: a style still loading must not throw away a route that computed
    // fine, which is exactly what happens when the user switches backdrop mid-request
    drawRoute();
  }

  /// Route rendering: a dark casing under a bright line, so the route stays readable over both
  /// the vector inspect colours and a hillshade.
  function drawRoute() {
    if (!mainMap || !trip) return;
    if (!mainMap.isStyleLoaded()) {
      whenStyleReady(mainMap, drawRoute);
      return;
    }
    const { line, maneuvers } = tripToGeoJson(trip);
    for (const [id, data] of [["route", line], ["route-maneuvers", maneuvers]]) {
      const existing = mainMap.getSource(id);
      if (existing) {
        existing.setData(data);
        continue;
      }
      try {
        mainMap.addSource(id, { type: "geojson", data });
      } catch {
        continue;
      }
      if (id === "route") {
        mainMap.addLayer({
          id: "route-casing", type: "line", source: id,
          layout: { "line-cap": "round", "line-join": "round" },
          paint: { "line-color": "#0b1a33", "line-width": 8, "line-opacity": 0.65 },
        });
        mainMap.addLayer({
          id: "route-line", type: "line", source: id,
          layout: { "line-cap": "round", "line-join": "round" },
          paint: { "line-color": "#4aa3ff", "line-width": 3.5 },
        });
      } else {
        mainMap.addLayer({
          id: "route-maneuver-dots", type: "circle", source: id,
          paint: {
            "circle-color": "#ffd166", "circle-radius": 3.5,
            "circle-stroke-width": 1, "circle-stroke-color": "#0b1a33",
          },
        });
      }
    }
  }

  /// Elevation profile of the computed route, sampled along its actual geometry rather than the
  /// straight lines between waypoints.
  async function profileRoute() {
    if (!trip || !terrainArt) return;
    profileBusy = true;
    error = "";
    try {
      profile = await invoke("elevation_profile", {
        req: { path: terrainArt.path, line: trip.points, densifyM: 50, thresholdM: 3 },
      });
    } catch (err) {
      error = String(err);
    } finally {
      profileBusy = false;
    }
  }

  function clearRoute() {
    drawn = [];
    trip = null;
    profile = null;
    drawOverlay();
    for (const id of ["route", "route-maneuvers"]) {
      if (mainMap?.getSource(id)) mainMap.getSource(id).setData({ type: "FeatureCollection", features: [] });
    }
  }

  async function computeProfile() {
    if (drawn.length < 2 || !terrainArt) return;
    profileBusy = true; error = "";
    try {
      profile = await invoke("elevation_profile", {
        req: { path: terrainArt.path, line: drawn, densifyM: 50, thresholdM: 3 },
      });
    } catch (err) { error = String(err); }
    finally { profileBusy = false; }
  }
</script>

<div class="bar">
  <select value={areaName} onchange={(e) => (areaName = e.target.value)}>
    {#each areas as a}<option value={a.name}>{a.name}</option>{/each}
  </select>

  <select onchange={(e) => { const f = e.target.value; if (f) { addArtifact(available.find((a) => a.file_name === f)); e.target.value = ""; } }}>
    <option value="">add layer…</option>
    {#each available as a}<option value={a.file_name}>{a.file_name}</option>{/each}
  </select>
  <select bind:value={addTo} title="which side new layers go to">
    <option value="main">→ right</option>
    <option value="secondary">→ left</option>
  </select>

  <select value={backdrop} onchange={(e) => setBackdrop(e.target.value)}>
    <option value="">no backdrop</option>
    {#each Object.entries(BACKDROPS) as [key, b]}<option value={key}>{b.label}</option>{/each}
  </select>

  <div class="seg" role="group" aria-label="geometry filter">
    {#each [["all", "All"], ["polygons", "Poly"], ["lines", "Lines"], ["points", "Points"]] as [id, label]}
      <button class:on={geomFilter === id} onclick={() => (geomFilter = id)}>{label}</button>
    {/each}
  </div>

  <div class="spacer"></div>
  <button class:on={comparing} onclick={toggleCompare}
          disabled={!secondarySources.length && !comparing}>Compare</button>
  <button class:on={mode === "inspect"} onclick={() => (mode = "inspect")}>Inspect</button>
  <button class:on={mode === "draw"} onclick={() => (mode = "draw")}>Draw</button>
  <button class:on={mode === "route"} onclick={() => (mode = "route")}
          disabled={!routingReady} title={routingReady ? "" : "this build has no Valhalla linked"}>
    Route
  </button>
</div>

{#if error}<p class="warn">{error}</p>{/if}

<div class="layout">
  <aside>
    <SourcePanel sources={mainSources} title={comparing ? "Right" : "Layers"}
                 onToggleSource={toggleSource} onToggleLayer={toggleLayer} onOpacity={setOpacity}
                 onMove={(i, d) => moveIn("main", i, d)} onRemove={remove} onFit={fit} />
    {#if secondarySources.length}
      <SourcePanel sources={secondarySources} title="Left"
                   onToggleSource={toggleSource} onToggleLayer={toggleLayer} onOpacity={setOpacity}
                   onMove={(i, d) => moveIn("secondary", i, d)} onRemove={remove} onFit={fit} />
    {/if}
  </aside>

  <div class="maps" bind:this={containerEl} class:comparing>
    <div class="map" bind:this={secondaryEl}></div>
    <div class="map" bind:this={mainEl}></div>
  </div>
</div>

<div class="panels">
  {#if mode === "route"}
    <div class="row">
      <select bind:value={costing} onchange={() => drawn.length >= 2 && computeRoute()}>
        {#each COSTING_MODELS as c}<option value={c}>{c}</option>{/each}
      </select>
      <span class="muted">{drawn.length} waypoints</span>
      <button onclick={computeRoute} disabled={drawn.length < 2 || routeBusy}>
        {routeBusy ? "Routing…" : "Route"}
      </button>
      <button onclick={profileRoute} disabled={!trip || !terrainArt || profileBusy}>
        {profileBusy ? "Sampling…" : "Elevation profile"}
      </button>
      <button class="ghost" onclick={clearRoute} disabled={!drawn.length}>Clear</button>
    </div>
    {#if trip}
      <p class="summary">
        <strong>{trip.lengthKm.toFixed(1)} km</strong>
        &middot; {formatDuration(trip.timeS)}
        &middot; <span class="muted">{trip.maneuvers.length} maneuvers</span>
      </p>
      <Profile {profile} />
      <ol class="maneuvers">
        {#each trip.maneuvers as m}
          <li><span class="km">{m.lengthKm.toFixed(2)} km</span> {m.instruction}</li>
        {/each}
      </ol>
    {:else}
      <p class="muted hint">Click two or more points on the map to route between them.</p>
    {/if}
  {:else if mode === "draw"}
    <div class="row">
      <strong>{drawn.length}</strong> <span class="muted">points</span>
      <button onclick={computeProfile} disabled={drawn.length < 2 || !terrainArt || profileBusy}>
        {profileBusy ? "Sampling…" : "Elevation profile"}
      </button>
      <button class="ghost" onclick={() => { drawn = []; profile = null; drawOverlay(); }}
              disabled={!drawn.length}>Clear</button>
      {#if !terrainArt}<span class="warn">no terrain archive in this area</span>{/if}
    </div>
    <Profile {profile} />
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
    <p class="muted hint">Click a feature to inspect. Add layers and reorder them in the panel.</p>
  {/if}
</div>

<style>
  .bar { display: flex; gap: 6px; align-items: center; margin-bottom: 10px; flex-wrap: wrap; }
  .bar select { padding: 6px 8px; background: #12151a; border: 1px solid #303845;
                border-radius: 5px; color: #dde3ea; font: inherit; font-size: 12px; max-width: 200px; }
  .bar button { background: #262d38; color: #9aa5b1; padding: 6px 11px; font-size: 12px; }
  .bar button.on { background: #2d5f4a; color: #fff; }
  .seg { display: flex; gap: 1px; }
  .seg button { border-radius: 0; padding: 6px 9px; }
  .seg button:first-child { border-radius: 5px 0 0 5px; }
  .seg button:last-child { border-radius: 0 5px 5px 0; }
  .spacer { flex: 1; }
  .layout { display: grid; grid-template-columns: 250px 1fr; gap: 10px; align-items: start; }
  aside { display: flex; flex-direction: column; gap: 12px; max-height: 520px; overflow: auto; }
  .maps { position: relative; height: 520px; border-radius: 8px; overflow: hidden;
          border: 1px solid #262d38; }
  .map { position: absolute; inset: 0; }
  /* Both maps stay laid out at all times. `display: none` on the idle one would leave the
     compare plugin measuring a container with no visible child; outside compare mode the main
     map simply covers it. */
  .maps:not(.comparing) .map:first-child { visibility: hidden; }
  :global(body.swiping) { user-select: none; }
  .panels { margin-top: 12px; }
  .row { display: flex; gap: 10px; align-items: center; margin-bottom: 10px; }
  .feat { border-top: 1px solid #262d38; padding-top: 8px; margin-top: 8px; }
  h4 { margin: 0 0 4px; font-size: 13px; }
  table { width: 100%; border-collapse: collapse; font-size: 12px; }
  td { padding: 2px 6px 2px 0; vertical-align: top; }
  td:first-child { color: #6b7684; width: 34%; }
  .muted { color: #6b7684; }
  .hint { font-size: 13px; }
  .summary { margin: 6px 0; font-size: 14px; }
  .maneuvers { margin: 8px 0 0; padding-left: 0; list-style: none; max-height: 200px;
               overflow: auto; font-size: 12px; color: #9aa5b1; }
  .maneuvers li { padding: 2px 0; border-bottom: 1px solid #1e242d; }
  .km { display: inline-block; width: 62px; text-align: right; color: #6b7684;
        font-variant-numeric: tabular-nums; margin-right: 8px; }
  .row select { padding: 6px 8px; background: #12151a; border: 1px solid #303845;
                border-radius: 5px; color: #dde3ea; font: inherit; font-size: 12px; }
  .warn { color: #d99a5b; }
</style>
