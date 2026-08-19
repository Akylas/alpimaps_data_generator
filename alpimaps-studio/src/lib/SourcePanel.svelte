<script>
  // Per-source controls: order, visibility, opacity, and per-layer toggles.
  let {
    sources = [],
    title = "",
    onToggleSource, onToggleLayer, onOpacity, onMove, onRemove, onFit,
  } = $props();

  let expanded = $state(new Set());

  function toggleExpanded(id) {
    const next = new Set(expanded);
    next.has(id) ? next.delete(id) : next.add(id);
    expanded = next;
  }
</script>

<div class="panel">
  {#if title}<h4>{title}</h4>{/if}
  {#if !sources.length}
    <p class="empty">nothing added</p>
  {/if}
  {#each sources as source, index (source.id)}
    <div class="source" class:off={!source.visible}>
      <div class="head">
        <input type="checkbox" checked={source.visible} title="show this file"
               onchange={() => onToggleSource(source)} />
        <button class="name" onclick={() => toggleExpanded(source.id)} title={source.file}>
          {source.file}
        </button>
        <span class="badge">{source.terrain ? source.demEncoding ?? "dem" : source.tileEncoding}</span>
        <button class="icon" onclick={() => onMove(index, -1)} disabled={index === 0} title="move up">↑</button>
        <button class="icon" onclick={() => onMove(index, 1)} disabled={index === sources.length - 1} title="move down">↓</button>
        <button class="icon" onclick={() => onFit(source)} title="zoom to bounds">⤢</button>
        <button class="icon" onclick={() => onRemove(source)} title="remove">×</button>
      </div>

      <input class="opacity" type="range" min="0" max="1" step="0.05" value={source.opacity}
             oninput={(e) => onOpacity(source, parseFloat(e.target.value))} />

      {#if expanded.has(source.id) && source.vector}
        <div class="layers">
          {#each source.layers as layer}
            <label class="layer">
              <input type="checkbox" checked={layer.visible}
                     onchange={() => onToggleLayer(source, layer)} />
              <span class="swatch" style="background:{layer.color}"></span>
              <span class="lname">{layer.id}</span>
              {#if layer.minzoom != null}<span class="z">z{layer.minzoom}–{layer.maxzoom}</span>{/if}
            </label>
          {/each}
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .panel { display: flex; flex-direction: column; gap: 6px; }
  h4 { font-size: 11px; text-transform: uppercase; letter-spacing: .05em; color: #6b7684;
       margin: 0 0 2px; }
  .empty { color: #5d6673; font-size: 12px; margin: 0; }
  .source { background: #161b22; border: 1px solid #262d38; border-radius: 6px; padding: 6px 8px; }
  .source.off { opacity: .5; }
  .head { display: flex; align-items: center; gap: 5px; }
  .name { flex: 1; background: none; border: 0; color: #dde3ea; font: inherit; font-size: 12px;
          text-align: left; padding: 0; cursor: pointer; overflow: hidden;
          text-overflow: ellipsis; white-space: nowrap; }
  .badge { background: #262d38; color: #8a94a2; font-size: 10px; padding: 1px 5px; border-radius: 3px; }
  .icon { background: none; border: 0; color: #6b7684; font-size: 13px; padding: 0 3px;
          cursor: pointer; line-height: 1; }
  .icon:disabled { opacity: .3; cursor: default; }
  .opacity { width: 100%; margin: 5px 0 0; height: 3px; }
  .layers { margin-top: 6px; max-height: 190px; overflow: auto; display: flex;
            flex-direction: column; gap: 1px; }
  .layer { display: flex; align-items: center; gap: 6px; font-size: 11px; color: #9aa5b1;
           padding: 1px 0; }
  .swatch { width: 9px; height: 9px; border-radius: 2px; flex: none; }
  .lname { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .z { color: #5d6673; font-size: 10px; }
  input[type="checkbox"] { margin: 0; }
</style>
