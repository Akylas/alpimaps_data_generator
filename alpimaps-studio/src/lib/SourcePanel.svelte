<script>
  // Per-source controls: order, visibility, opacity, terrain mode, and layer toggles.
  import { TERRAIN_MODES, layerSummary } from "./sources.js";

  let {
    sources = [], title = "", collapsed = false,
    onToggleSource, onToggleLayer, onSetAllLayers, onOpacity, onMove, onRemove, onFit,
    onTerrainMode, onAdd, addable = [],
  } = $props();

  let expanded = $state(new Set());
  let adding = $state(false);

  function toggleExpanded(id) {
    const next = new Set(expanded);
    next.has(id) ? next.delete(id) : next.add(id);
    expanded = next;
  }

  const KIND_LABEL = {
    basemap: "Basemap", routes: "Routes", terrain_rgb: "Terrain",
    hillshade: "Raster", unknown: "Other",
  };

  let grouped = $derived.by(() => {
    const by = new Map();
    for (const a of addable) {
      const key = KIND_LABEL[a.kind] ?? "Other";
      if (!by.has(key)) by.set(key, []);
      by.get(key).push(a);
    }
    return [...by.entries()];
  });
</script>

<div class="panel" class:collapsed>
  <header>
    <h4>{title}</h4>
    <button class="add" onclick={() => (adding = !adding)} title="add a layer">+</button>
  </header>

  {#if adding}
    <div class="adder">
      {#each grouped as [group, items]}
        <div class="group">{group}</div>
        {#each items as art}
          <button class="option" onclick={() => { onAdd(art); adding = false; }}>
            {art.file_name}
            {#if art.variant}<span class="tag">{art.variant}</span>{/if}
          </button>
        {/each}
      {/each}
      {#if !grouped.length}<p class="empty">everything is already added</p>{/if}
    </div>
  {/if}

  {#if !sources.length}
    <p class="empty">no layers — use + to add one</p>
  {/if}

  {#each sources as source, index (source.id)}
    {@const summary = layerSummary(source)}
    <div class="source" class:off={!source.visible}>
      <div class="head">
        <input type="checkbox" checked={source.visible} title="show this file"
               onchange={() => onToggleSource(source)} />
        <button class="name" onclick={() => toggleExpanded(source.id)} title={source.file}>
          {source.file}
        </button>
        <div class="actions">
          <button class="icon" onclick={() => onMove(index, -1)} disabled={index === 0} title="move up">↑</button>
          <button class="icon" onclick={() => onMove(index, 1)} disabled={index === sources.length - 1} title="move down">↓</button>
          <button class="icon" onclick={() => onFit(source)} title="zoom to bounds">⤢</button>
          <button class="icon danger" onclick={() => onRemove(source)} title="remove">×</button>
        </div>
      </div>

      <div class="meta">
        <span class="badge">{source.terrain ? source.demEncoding ?? "dem" : source.tileEncoding}</span>
        {#if source.vector}
          <button class="link" onclick={() => toggleExpanded(source.id)}>
            {summary.on}/{summary.total} layers
          </button>
        {/if}
        <input class="opacity" type="range" min="0" max="1" step="0.05" value={source.opacity}
               title="opacity" oninput={(e) => onOpacity(source, parseFloat(e.target.value))} />
      </div>

      {#if source.terrain}
        <div class="modes" role="group" aria-label="terrain rendering">
          {#each TERRAIN_MODES as mode}
            <button class:on={source.terrainMode === mode}
                    onclick={() => onTerrainMode(source, mode)}>
              {mode === "terrain3d" ? "3D" : mode}
            </button>
          {/each}
        </div>
      {/if}

      {#if expanded.has(source.id) && source.vector}
        <div class="bulk">
          <button class="link" onclick={() => onSetAllLayers(source, true)}>all</button>
          <button class="link" onclick={() => onSetAllLayers(source, false)}>none</button>
        </div>
        <div class="layers">
          {#each source.layers as layer}
            <label class="layer">
              <input type="checkbox" checked={layer.visible}
                     onchange={() => onToggleLayer(source, layer)} />
              <span class="swatch" style="background:{layer.color}"></span>
              <span class="lname">{layer.id}</span>
              {#if layer.minzoom != null}<span class="z">{layer.minzoom}–{layer.maxzoom}</span>{/if}
            </label>
          {/each}
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .panel { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .panel.collapsed { display: none; }
  header { display: flex; align-items: center; justify-content: space-between; }
  h4 { font-size: 11px; text-transform: uppercase; letter-spacing: .06em; color: #6b7684; margin: 0; }
  .add { background: #262d38; color: #9aa5b1; border: 0; border-radius: 4px; width: 20px;
         height: 20px; line-height: 1; padding: 0; cursor: pointer; font-size: 14px; }
  .adder { background: #12151a; border: 1px solid #303845; border-radius: 6px; padding: 6px;
           max-height: 230px; overflow: auto; }
  .group { font-size: 10px; text-transform: uppercase; color: #5d6673; margin: 4px 0 2px; }
  .option { display: block; width: 100%; text-align: left; background: none; border: 0;
            color: #dde3ea; font: inherit; font-size: 12px; padding: 3px 4px; border-radius: 3px;
            cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .option:hover { background: #1f2630; }
  .empty { color: #5d6673; font-size: 12px; margin: 4px 0; }
  .source { background: #161b22; border: 1px solid #262d38; border-radius: 6px; padding: 7px 8px; }
  .source.off { opacity: .5; }
  .head { display: flex; align-items: center; gap: 6px; }
  .name { flex: 1; background: none; border: 0; color: #dde3ea; font: inherit; font-size: 12px;
          text-align: left; padding: 0; cursor: pointer; overflow: hidden;
          text-overflow: ellipsis; white-space: nowrap; }
  .actions { display: flex; gap: 1px; opacity: .55; }
  .source:hover .actions { opacity: 1; }
  .icon { background: none; border: 0; color: #7c8896; font-size: 12px; padding: 0 3px;
          cursor: pointer; line-height: 1; }
  .icon.danger:hover { color: #e6584d; }
  .icon:disabled { opacity: .3; cursor: default; }
  .meta { display: flex; align-items: center; gap: 8px; margin-top: 5px; }
  .badge { background: #262d38; color: #8a94a2; font-size: 10px; padding: 1px 5px; border-radius: 3px; }
  .link { background: none; border: 0; color: #7c8896; font: inherit; font-size: 11px;
          padding: 0; cursor: pointer; text-decoration: underline; }
  .opacity { flex: 1; height: 3px; min-width: 40px; }
  .modes { display: flex; gap: 1px; margin-top: 6px; }
  .modes button { flex: 1; background: #12151a; border: 1px solid #303845; color: #8a94a2;
                  font-size: 10px; padding: 3px 0; cursor: pointer; }
  .modes button:first-child { border-radius: 4px 0 0 4px; }
  .modes button:last-child { border-radius: 0 4px 4px 0; }
  .modes button.on { background: #2d5f4a; border-color: #2d5f4a; color: #fff; }
  .bulk { display: flex; gap: 8px; margin-top: 6px; }
  .layers { margin-top: 4px; max-height: 210px; overflow: auto; display: flex;
            flex-direction: column; gap: 1px; }
  .layer { display: flex; align-items: center; gap: 6px; font-size: 11px; color: #9aa5b1; }
  .swatch { width: 9px; height: 9px; border-radius: 2px; flex: none; }
  .lname { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .z { color: #5d6673; font-size: 10px; }
  input[type="checkbox"] { margin: 0; }
</style>
