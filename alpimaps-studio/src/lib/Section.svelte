<script>
  // A collapsible block. Settings that are rarely touched should not push the thing you came
  // for below the fold.
  import { untrack } from "svelte";

  let { title = "", subtitle = "", open = true, badge = "", children } = $props();
  // `open` is the initial state, not a binding: reading it untracked says so and keeps Svelte
  // from warning that only the first value is captured.
  let expanded = $state(untrack(() => open));
</script>

<section class="card">
  <button class="head" onclick={() => (expanded = !expanded)} aria-expanded={expanded}>
    <span class="chev" class:open={expanded}>›</span>
    <h3>{title}</h3>
    {#if badge}<span class="badge">{badge}</span>{/if}
    {#if subtitle}<span class="subtitle">{subtitle}</span>{/if}
  </button>
  {#if expanded}
    <div class="body">{@render children?.()}</div>
  {/if}
</section>

<style>
  .card { background: #1a1f27; border: 1px solid #262d38; border-radius: 8px; margin-bottom: 10px; }
  .head { display: flex; align-items: center; gap: 8px; width: 100%; background: none; border: 0;
          padding: 11px 14px; cursor: pointer; text-align: left; color: inherit; }
  .chev { color: #6b7684; transition: transform .12s; display: inline-block; font-size: 14px; }
  .chev.open { transform: rotate(90deg); }
  h3 { font-size: 12px; text-transform: uppercase; letter-spacing: .06em; color: #9aa5b1;
       margin: 0; font-weight: 600; }
  .badge { background: #2d5f4a; color: #cfe8db; font-size: 10px; padding: 1px 7px; border-radius: 10px; }
  .subtitle { color: #5d6673; font-size: 12px; margin-left: auto; overflow: hidden;
              text-overflow: ellipsis; white-space: nowrap; }
  .body { padding: 0 14px 14px; }
</style>
