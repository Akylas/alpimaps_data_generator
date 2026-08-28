<script>
  // Render an archive through a real MapLibre style, editable in place.
  //
  // The point is testing a style against local tiles: whatever the style names its sources, they
  // are repointed at the local server, so a cartographic style written for a hosted tileset
  // renders the archive on disk without editing its URLs by hand.
  let { sources = [], base = "", onApply, onClear, applied = false } = $props();

  let text = $state("");
  let error = $state("");
  let busy = $state("");
  let target = $state("");
  let styleUrl = $state("");

  /// MapTiler is the quickest way to get a real, complete OpenMapTiles style in front of these
  /// tiles - its styles name their vector source `openmaptiles`, which is the schema the basemap
  /// step produces. Only the vector styles are listed: a satellite raster has nothing to repoint.
  const MAPTILER_STYLES = [
    ["streets-v2", "Streets"],
    ["outdoor-v2", "Outdoor"],
    ["topo-v2", "Topo"],
    ["winter-v2", "Winter"],
    ["basic-v2", "Basic"],
    ["bright-v2", "Bright"],
    ["dataviz", "Dataviz"],
    ["landscape", "Landscape"],
    ["ocean", "Ocean"],
  ];

  /// The key stays in this browser profile rather than in the app's settings file: it is a
  /// personal credential for a preview, not part of how the pipeline builds anything.
  const KEY_STORE = "maptilerKey";
  let maptilerKey = $state(readKey());
  let maptilerStyle = $state("outdoor-v2");

  function readKey() {
    try { return localStorage.getItem(KEY_STORE) ?? ""; } catch { return ""; }
  }
  function saveKey(value) {
    maptilerKey = value;
    try { localStorage.setItem(KEY_STORE, value); } catch {}
  }

  const STARTER = {
    version: 8,
    sources: {},
    layers: [
      { id: "bg", type: "background", paint: { "background-color": "#0f1115" } },
      {
        id: "water", type: "fill", source: "SOURCE", "source-layer": "water",
        paint: { "fill-color": "#1b3a5c" },
      },
      {
        id: "roads", type: "line", source: "SOURCE", "source-layer": "transportation",
        paint: { "line-color": "#c8a86b", "line-width": ["interpolate", ["linear"], ["zoom"], 6, 0.4, 14, 2] },
      },
    ],
  };

  /// Point at the first vector archive as soon as there is one. Left blank the select reads as
  /// "nothing chosen" while Apply would silently have used that same archive anyway.
  $effect(() => {
    const vector = sources.filter((s) => s.vector);
    if (vector.some((s) => s.file === target)) return;
    // the basemap first: a full cartographic style is written against that schema, and pointing
    // it at the routes archive draws one layer and looks like the style is broken
    target = (vector.find((s) => s.artifact?.kind === "basemap") ?? vector[0])?.file ?? "";
  });

  function starter() {
    text = JSON.stringify(STARTER, null, 2);
    error = "";
  }

  /// Fetch and apply in one go. Loading a style into the box and leaving it there is a step
  /// nobody wanted: the reason to pick a style is to see it.
  async function loadFrom(url, what) {
    error = "";
    busy = what;
    try {
      const res = await fetch(url);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      text = JSON.stringify(await res.json(), null, 2);
      apply();
    } catch (e) {
      error = `could not load style: ${e.message ?? e}`;
    } finally {
      busy = "";
    }
  }

  function loadMaptiler() {
    if (!maptilerKey.trim()) {
      error = "MapTiler needs an API key - make one at maptiler.com, it is free for a few thousand loads";
      return;
    }
    loadFrom(
      `https://api.maptiler.com/maps/${maptilerStyle}/style.json?key=${encodeURIComponent(maptilerKey.trim())}`,
      "maptiler",
    );
  }

  function loadUrl() {
    const url = styleUrl.trim();
    if (!url) {
      // this used to be a window.prompt(), which the app's webview does not implement at all -
      // the button simply did nothing, with no error anywhere
      error = "paste a style.json URL first";
      return;
    }
    loadFrom(url, "url");
  }

  function apply() {
    error = "";
    let style;
    try {
      style = JSON.parse(text);
    } catch (e) {
      error = `not valid JSON: ${e.message}`;
      return;
    }
    const source = sources.find((s) => s.file === target) ?? sources.find((s) => s.vector);
    if (!source) {
      error = "no vector source to point the style at - add a basemap layer to the map first";
      return;
    }
    onApply(style, source);
  }
</script>

<div class="editor">
  <div class="pickers">
    <div class="pick">
      <span class="lbl">MapTiler</span>
      <select bind:value={maptilerStyle} title="MapTiler style">
        {#each MAPTILER_STYLES as [id, label]}<option value={id}>{label}</option>{/each}
      </select>
      <input class="key" type="password" placeholder="API key" autocomplete="off"
             value={maptilerKey} oninput={(e) => saveKey(e.target.value)}
             title="stored in this app only, and sent to MapTiler when loading a style" />
      <button onclick={loadMaptiler} disabled={!!busy}>
        {busy === "maptiler" ? "Loading…" : "Load"}
      </button>
    </div>

    <div class="pick">
      <span class="lbl">Style URL</span>
      <input bind:value={styleUrl} spellcheck="false" placeholder="https://…/style.json"
             onkeydown={(e) => e.key === "Enter" && loadUrl()} />
      <button class="ghost" onclick={loadUrl} disabled={!!busy}>
        {busy === "url" ? "Loading…" : "Load"}
      </button>
      <button class="ghost" onclick={starter} title="a three-layer style to edit from">Starter</button>
    </div>
  </div>

  <div class="row">
    <span class="lbl">Archive</span>
    <select bind:value={target} title="the archive every vector source is repointed at">
      {#each sources.filter((s) => s.vector) as s}<option value={s.file}>{s.file}</option>{/each}
    </select>
    <div class="spacer"></div>
    <button onclick={apply} disabled={!text.trim()}>Apply</button>
    <button class="ghost" onclick={onClear} disabled={!applied}>Back to inspect</button>
  </div>

  {#if error}<p class="warn">{error}</p>{/if}
  <textarea bind:value={text} spellcheck="false"
            placeholder="Pick a MapTiler style, paste a style.json URL, or press Starter. Whatever loads here is editable, and every vector source in it is repointed at the archive selected above."></textarea>
  <p class="hint">
    Ctrl/Cmd+Enter applies. A loaded style renders your own tiles: its <code>url</code> and
    <code>tiles</code> entries are rewritten to the local server, while fonts and sprites still
    come from wherever the style points them.
  </p>
</div>

<svelte:window on:keydown={(e) => { if ((e.metaKey || e.ctrlKey) && e.key === "Enter") apply(); }} />

<style>
  .editor { display: flex; flex-direction: column; gap: 6px; height: 100%; }
  .pickers { display: flex; flex-direction: column; gap: 5px; padding: 7px 8px; background: var(--surface);
             border: 1px solid var(--line-2); border-radius: var(--r); }
  .pick, .row { display: flex; gap: 6px; align-items: center; }
  .lbl { font-size: 10px; text-transform: uppercase; letter-spacing: .07em; color: var(--faint);
         width: 66px; flex: none; }
  .spacer { flex: 1; }
  select, input { padding: 5px 8px; background: var(--bg); border: 1px solid var(--border);
                  border-radius: 5px; color: var(--text); font: inherit; font-size: 12px; }
  select { max-width: 220px; }
  .pick input { flex: 1; min-width: 0; }
  .key { max-width: 200px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
  button { font-size: 12px; padding: 5px 11px; }
  textarea { flex: 1; min-height: 180px; background: var(--bg); border: 1px solid var(--border);
             border-radius: 6px; color: var(--text); padding: 8px; font-family: ui-monospace, monospace;
             font-size: 11px; line-height: 1.45; resize: vertical; }
  .hint { color: var(--faint); font-size: 11px; margin: 0; }
  .warn { color: var(--warn); font-size: 12px; margin: 0; }
</style>
