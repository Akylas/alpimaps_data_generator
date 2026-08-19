<script>
  import { invoke } from "./api.js";
  import { mb, pct, KIND_LABEL, formatLabel } from "./format.js";

  let areas = $state([]);
  let active = $state(0);
  let error = $state("");
  let stats = $state({});
  let loading = $state({});

  let compareA = $state("");
  let compareB = $state("");
  let comparison = $state(null);
  let comparing = $state(false);

  export async function refresh() {
    error = "";
    try {
      areas = await invoke("list_areas");
      if (active >= areas.length) active = 0;
    } catch (err) {
      error = String(err);
    }
  }

  refresh();

  let area = $derived(areas[active]);

  async function loadStats(artifact) {
    if (stats[artifact.path] || loading[artifact.path]) return;
    loading = { ...loading, [artifact.path]: true };
    try {
      stats = { ...stats, [artifact.path]: await invoke("artifact_stats", { path: artifact.path }) };
    } catch (err) {
      stats = { ...stats, [artifact.path]: { error: String(err) } };
    } finally {
      loading = { ...loading, [artifact.path]: false };
    }
  }

  async function runCompare() {
    if (!compareA || !compareB || compareA === compareB) return;
    comparing = true;
    comparison = null;
    try {
      comparison = await invoke("compare_artifacts", { area: area.name, a: compareA, b: compareB });
    } catch (err) {
      comparison = { error: String(err) };
    } finally {
      comparing = false;
    }
  }
</script>

<div class="head">
  <div class="tabs">
    {#each areas as a, i}
      <button class:active={i === active} onclick={() => (active = i)}>
        {a.name}<span class="size">{mb(a.artifacts.reduce((s, x) => s + x.size_bytes, 0))}</span>
      </button>
    {/each}
  </div>
  <button class="ghost" onclick={refresh}>Rescan</button>
</div>

{#if error}
  <p class="warn">{error}</p>
{:else if !areas.length}
  <p class="muted">No areas found under the configured output root.</p>
{/if}

{#if area}
  <table>
    <thead>
      <tr><th>File</th><th>Kind</th><th>Format</th><th>Zooms</th><th class="num">Size</th><th></th></tr>
    </thead>
    <tbody>
      {#each area.artifacts as art}
        <tr class:variant={art.variant}>
          <td>
            <code>{art.file_name}</code>
            {#if art.variant}<span class="tag">{art.variant}</span>{/if}
            {#if art.probe_error}<span class="tag err">unreadable</span>{/if}
          </td>
          <td>{KIND_LABEL[art.kind] ?? art.kind}</td>
          <td>
            {formatLabel(art.format)}
            {#if art.encoding}<span class="tag">{art.encoding}</span>{/if}
          </td>
          <td>{art.minzoom ?? "—"}–{art.maxzoom ?? "—"}</td>
          <td class="num">{mb(art.size_bytes)}</td>
          <td><button class="ghost" onclick={() => loadStats(art)} disabled={loading[art.path]}>
            {loading[art.path] ? "…" : stats[art.path] ? "↻" : "stats"}
          </button></td>
        </tr>
        {#if stats[art.path]}
          {@const s = stats[art.path]}
          <tr class="detail">
            <td colspan="6">
              {#if s.error}
                <span class="warn">{s.error}</span>
              {:else}
                <p>
                  {s.addressed_tiles.toLocaleString()} tiles addressed &middot; {mb(s.addressed_bytes)}
                  {#if s.unique_tiles != null}
                    &nbsp;|&nbsp; {s.unique_tiles.toLocaleString()} unique &middot; {mb(s.unique_bytes)}
                    <span class="tag">dedup {((1 - s.unique_tiles / s.addressed_tiles) * 100).toFixed(1)}%</span>
                  {/if}
                </p>
                <div class="zoombars">
                  {#each s.per_zoom.slice().reverse() as z}
                    {@const share = z.bytes / s.addressed_bytes}
                    <div class="zoomrow">
                      <span class="z">z{z.zoom}</span>
                      <span class="bar"><i style="width:{(share * 100).toFixed(1)}%"></i></span>
                      <span class="v">{mb(z.bytes)}</span>
                      <span class="n">{z.tiles.toLocaleString()}</span>
                    </div>
                  {/each}
                </div>
              {/if}
            </td>
          </tr>
        {/if}
        {#if art.layers?.length}
          <tr class="detail muted"><td colspan="6">
            {art.layers.length} layers: {art.layers.map((l) => l.id).join(", ")}
          </td></tr>
        {/if}
      {/each}
    </tbody>
  </table>

  <section class="compare">
    <h3>Compare two builds</h3>
    <div class="row">
      <select bind:value={compareA}>
        <option value="">A…</option>
        {#each area.artifacts as art}<option value={art.path}>{art.file_name}</option>{/each}
      </select>
      <select bind:value={compareB}>
        <option value="">B…</option>
        {#each area.artifacts as art}<option value={art.path}>{art.file_name}</option>{/each}
      </select>
      <button onclick={runCompare} disabled={comparing || !compareA || !compareB || compareA === compareB}>
        {comparing ? "Comparing…" : "Compare"}
      </button>
    </div>

    {#if comparison?.error}
      <p class="warn">{comparison.error}</p>
    {:else if comparison}
      <p class="muted">
        file size {mb(comparison.size_a)} → {mb(comparison.size_b)}
        <strong class={comparison.size_b <= comparison.size_a ? "ok" : "warn"}>
          {pct(((comparison.size_b - comparison.size_a) / comparison.size_a) * 100)}
        </strong>
      </p>
      <table class="cmp">
        <thead><tr><th>Zoom</th><th class="num">A</th><th class="num">B</th><th class="num">Δ</th><th class="num">tiles A→B</th></tr></thead>
        <tbody>
          {#each comparison.zooms.slice().reverse() as z}
            {@const change = z.bytes_a > 0 ? ((z.bytes_b - z.bytes_a) / z.bytes_a) * 100 : null}
            <tr>
              <td>z{z.zoom}</td>
              <td class="num">{mb(z.bytes_a)}</td>
              <td class="num">{mb(z.bytes_b)}</td>
              <td class="num" class:ok={change < 0} class:warn={change > 0}>{pct(change)}</td>
              <td class="num muted">{z.tiles_a.toLocaleString()} → {z.tiles_b.toLocaleString()}</td>
            </tr>
          {/each}
        </tbody>
      </table>

      {#if comparison.layers.length}
        <h4>Layer fields</h4>
        {#each comparison.layers as l}
          <p class="muted">
            <code>{l.layer}</code>
            {#if l.only_in_a_fields.length}<span class="warn">−{l.only_in_a_fields.join(" −")}</span>{/if}
            {#if l.only_in_b_fields.length}<span class="ok">+{l.only_in_b_fields.join(" +")}</span>{/if}
          </p>
        {/each}
      {/if}

      {#if Object.keys(comparison.metadata.changed).length}
        <h4>Metadata</h4>
        {#each Object.entries(comparison.metadata.changed) as [key, [va, vb]]}
          <p class="muted"><code>{key}</code> {va} → {vb}</p>
        {/each}
      {/if}
    {/if}
  </section>
{/if}

<style>
  .head { display: flex; align-items: center; gap: 10px; margin-bottom: 14px; }
  .tabs { display: flex; gap: 4px; flex: 1; flex-wrap: wrap; }
  .tabs button { background: #1a1f27; border: 1px solid #262d38; color: #9aa5b1; padding: 6px 12px; }
  .tabs button.active { background: #2d5f4a; border-color: #2d5f4a; color: #fff; }
  .size { color: #6b7684; margin-left: 8px; font-size: 12px; }
  .tabs button.active .size { color: #b9dcc9; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th { text-align: left; color: #6b7684; font-weight: 500; font-size: 11px;
       text-transform: uppercase; letter-spacing: .05em; padding: 6px 8px; border-bottom: 1px solid #262d38; }
  td { padding: 7px 8px; border-bottom: 1px solid #1e242d; vertical-align: top; }
  .num { text-align: right; font-variant-numeric: tabular-nums; }
  tr.variant code { color: #8a94a2; }
  tr.detail td { background: #161b22; padding: 10px 12px; }
  .tag { display: inline-block; margin-left: 6px; padding: 1px 6px; border-radius: 3px;
         background: #262d38; color: #9aa5b1; font-size: 11px; }
  .tag.err { background: #4a2d2d; color: #d99a5b; }
  .zoombars { display: flex; flex-direction: column; gap: 2px; }
  .zoomrow { display: grid; grid-template-columns: 34px 1fr 74px 64px; gap: 8px; align-items: center;
             font-size: 12px; font-variant-numeric: tabular-nums; }
  .z { color: #7c8896; }
  .bar { background: #1e242d; height: 8px; border-radius: 2px; overflow: hidden; }
  .bar i { display: block; height: 100%; background: #3d7a5f; }
  .v { text-align: right; }
  .n { text-align: right; color: #6b7684; }
  .compare { margin-top: 22px; border-top: 1px solid #262d38; padding-top: 16px; }
  h3 { font-size: 13px; text-transform: uppercase; letter-spacing: .06em; color: #7c8896; margin: 0 0 12px; }
  h4 { font-size: 12px; color: #7c8896; margin: 16px 0 6px; }
  .row { display: flex; gap: 8px; margin-bottom: 12px; }
  select { flex: 1; padding: 7px 9px; background: #12151a; border: 1px solid #303845;
           border-radius: 5px; color: #dde3ea; font: inherit; }
  .cmp { margin-top: 8px; }
  .muted { color: #6b7684; }
  .ok { color: #7cc9a0; }
  .warn { color: #d99a5b; }
  p { margin: 4px 0; }
</style>
