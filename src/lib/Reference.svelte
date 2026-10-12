<script>
  // A grid of things you can click to read about, with the detail below.
  //
  // The CLI reference reads this way and it works: the whole surface is visible at a glance,
  // and the explanation appears where you are looking rather than in a wall of prose. Steps and
  // map modes use the same component so the docs read as one thing.
  let { items = [], initial = "", empty = "nothing here" } = $props();

  let picked = $state(initial);
  let current = $derived(items.find((i) => i.id === picked));
</script>

<div class="grid">
  {#each items as item}
    <button class="item" class:on={picked === item.id} class:off={item.muted}
            onclick={() => (picked = picked === item.id ? "" : item.id)}>
      <span class="name">{item.name}</span>
      <span class="about">{item.about}</span>
    </button>
  {/each}
  {#if !items.length}<p class="empty">{empty}</p>{/if}
</div>

{#if current}
  <div class="detail">
    <h4>{current.name}</h4>
    <p>{current.detail}</p>
    {#if current.facts?.length}
      <dl>
        {#each current.facts as [term, value]}
          {#if value}
            <dt>{term}</dt>
            <dd>{#if Array.isArray(value)}{#each value as v}<code>{v}</code>{/each}{:else}{value}{/if}</dd>
          {/if}
        {/each}
      </dl>
    {/if}
  </div>
{/if}

<style>
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 6px; }
  .item { display: flex; flex-direction: column; align-items: flex-start; gap: 3px;
          text-align: left; background: var(--surface); border: 1px solid var(--line-2);
          color: var(--text-2); padding: 9px 11px; border-radius: var(--r); }
  .item:hover { background: var(--hover); }
  .item.on { border-color: var(--accent); background: var(--accent-dim); color: var(--text); }
  .item.off { opacity: .6; }
  .name { font-size: 13px; color: var(--text); font-weight: 500; }
  .about { font-size: 11px; color: var(--muted-2); line-height: 1.45; }
  .empty { color: var(--faint); font-size: 12px; }
  .detail { margin-top: 12px; padding: 12px 14px; background: var(--bg);
            border: 1px solid var(--line-2); border-radius: var(--r); }
  .detail h4 { margin: 0 0 6px; font-size: 12px; text-transform: uppercase;
               letter-spacing: .06em; color: var(--muted-2); }
  .detail p { margin: 0; color: var(--text-2); font-size: 13px; line-height: 1.6;
              max-width: 80ch; }
  dl { display: grid; grid-template-columns: 96px 1fr; gap: 4px 12px; margin: 10px 0 0;
       font-size: 12px; }
  dt { color: var(--faint); text-transform: uppercase; font-size: 10px; letter-spacing: .06em;
       padding-top: 2px; }
  dd { margin: 0; color: var(--text-2); }
  dd code { display: block; color: var(--text-3); font-size: 11px; word-break: break-all; }
</style>
