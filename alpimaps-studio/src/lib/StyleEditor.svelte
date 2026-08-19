<script>
  // Render an archive through a real MapLibre style, editable in place.
  //
  // The point is testing a style against local tiles: whatever the style names its sources, they
  // are repointed at the local server, so a cartographic style written for a hosted tileset
  // renders the archive on disk without editing its URLs by hand.
  let { sources = [], base = "", onApply, onClear, applied = false } = $props();

  let text = $state("");
  let error = $state("");
  let target = $state("");

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

  function starter() {
    const vector = sources.find((s) => s.vector);
    target = vector?.file ?? "";
    text = JSON.stringify(STARTER, null, 2);
    error = "";
  }

  async function loadFrom(url) {
    error = "";
    try {
      const res = await fetch(url);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      text = JSON.stringify(await res.json(), null, 2);
    } catch (e) {
      error = `could not load style: ${e.message ?? e}`;
    }
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
      error = "no vector source to point the style at";
      return;
    }
    onApply(style, source);
  }
</script>

<div class="editor">
  <div class="row">
    <select bind:value={target}>
      {#each sources.filter((s) => s.vector) as s}<option value={s.file}>{s.file}</option>{/each}
    </select>
    <button class="ghost" onclick={starter}>Starter</button>
    <button class="ghost" onclick={() => { const u = prompt("Style URL"); if (u) loadFrom(u); }}>
      Load URL…
    </button>
    <div class="spacer"></div>
    <button onclick={apply} disabled={!text.trim()}>Apply</button>
    <button class="ghost" onclick={onClear} disabled={!applied}>Back to inspect</button>
  </div>
  {#if error}<p class="warn">{error}</p>{/if}
  <textarea bind:value={text} spellcheck="false"
            placeholder="Paste a MapLibre style, or press Starter. Every vector source in it is repointed at the selected archive."></textarea>
  <p class="hint">
    Ctrl/Cmd+Enter applies. Sources are rewritten to the local server, so <code>url</code> and
    <code>tiles</code> entries in the style are ignored.
  </p>
</div>

<svelte:window on:keydown={(e) => { if ((e.metaKey || e.ctrlKey) && e.key === "Enter") apply(); }} />

<style>
  .editor { display: flex; flex-direction: column; gap: 6px; height: 100%; }
  .row { display: flex; gap: 6px; align-items: center; }
  .spacer { flex: 1; }
  select { padding: 5px 8px; background: #12151a; border: 1px solid #303845; border-radius: 5px;
           color: #dde3ea; font: inherit; font-size: 12px; max-width: 220px; }
  textarea { flex: 1; min-height: 220px; background: #12151a; border: 1px solid #303845;
             border-radius: 6px; color: #dde3ea; padding: 8px; font-family: ui-monospace, monospace;
             font-size: 11px; line-height: 1.45; resize: vertical; }
  .hint { color: #5d6673; font-size: 11px; margin: 0; }
  .warn { color: #d99a5b; font-size: 12px; margin: 0; }
</style>
