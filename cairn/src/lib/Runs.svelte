<script>
  /**
   * Every build in flight, whichever tab is open.
   *
   * Builds used to be one at a time and visible only from the Build tab, so a run was easy to
   * lose track of: switch to the map, or to another area, and the only sign of an hour-long
   * basemap was the fan. Now that areas build concurrently there can be several at once, and
   * each needs its own progress and its own Cancel - a global one would have to guess.
   */
  import { invoke, listen } from "./api.js";
  import { onMount } from "svelte";

  let { onOpen } = $props();

  let runs = $state([]);
  let labels = $state({});
  /// Ticks once a second, so "running for" stays honest without the backend re-sending it.
  let now = $state(Math.floor(Date.now() / 1000));

  onMount(async () => {
    try {
      for (const s of await invoke("list_steps", {})) labels[s.id] = s.label;
    } catch {}
    await refresh();
    // `step` fires many times a second during a build; the snapshot is only worth re-reading at
    // about the rate a human reads it
    let due = null;
    const soon = () => {
      if (due) return;
      due = setTimeout(() => { due = null; refresh(); }, 500);
    };
    const offStep = await listen("step", soon);
    const offRuns = await listen("runs-changed", refresh);
    const tick = setInterval(() => (now = Math.floor(Date.now() / 1000)), 1000);
    return () => { offStep(); offRuns(); clearInterval(tick); clearTimeout(due); };
  });

  async function refresh() {
    try { runs = await invoke("active_runs"); }
    catch { runs = []; }
  }

  async function cancel(area) {
    try { await invoke("cancel_run", { area }); }
    catch {}
    await refresh();
  }

  const labelFor = (id) => labels[id] ?? id;

  function elapsed(since) {
    const secs = Math.max(0, now - since);
    if (secs >= 3600) return `${Math.floor(secs / 3600)}h${Math.floor((secs % 3600) / 60)}m`;
    if (secs >= 60) return `${Math.floor(secs / 60)}m${secs % 60}s`;
    return `${secs}s`;
  }
</script>

{#if runs.length}
  <div class="runs">
    {#each runs as run (run.area)}
      <div class="run" class:stopping={run.cancelling}>
        <button class="open" onclick={() => onOpen?.(run.area)} title={`open ${run.area} in Build`}>
          <span class="dot"></span>
          <strong>{run.area}</strong>
          <span class="what">
            {#if run.waitingFor}
              queued behind <code>{run.waitingFor}</code>
            {:else if run.step}
              {labelFor(run.step)}{run.phase ? ` · ${run.phase}` : ""}
            {:else}
              starting
            {/if}
          </span>
          <span class="count">{run.completed.length}/{run.planned.length}</span>
          <span class="time">{elapsed(run.startedAt)}</span>
        </button>
        <div class="bar"><div class="fill" style="width:{run.waitingFor ? 0 : run.percent}%"></div></div>
        <button class="ghost tiny" disabled={run.cancelling} onclick={() => cancel(run.area)}>
          {run.cancelling ? "stopping…" : "cancel"}
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .runs { display: flex; flex-direction: column; gap: 4px; padding: 6px 20px;
          border-bottom: 1px solid var(--line-2); background: var(--bg-sunken); }
  .run { display: flex; align-items: center; gap: 10px; max-width: 1100px; margin: 0 auto;
         width: 100%; position: relative; }
  .open { display: flex; align-items: center; gap: 8px; flex: 1; min-width: 0;
          background: none; border: 0; padding: 3px 0; color: var(--text-2); font: inherit;
          font-size: 12px; text-align: left; }
  .open:hover:not(:disabled) { background: none; color: var(--text); }
  .open strong { font-weight: 600; color: var(--text); }
  .open .what { color: var(--muted-2); overflow: hidden; text-overflow: ellipsis;
                white-space: nowrap; }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--ok); flex: none;
         animation: pulse 1.6s ease-in-out infinite; }
  .stopping .dot { background: var(--warn); }
  .count { margin-left: auto; color: var(--faint); font-variant-numeric: tabular-nums; }
  .time { color: var(--faint); font-variant-numeric: tabular-nums; width: 52px;
          text-align: right; }
  .bar { position: absolute; left: 0; right: 0; bottom: 0; height: 2px; background: var(--line-2);
         border-radius: 2px; overflow: hidden; }
  .fill { height: 100%; background: var(--accent-hi); transition: width .2s ease; }
  .tiny { padding: 1px 8px; font-size: 10px; }
  @keyframes pulse { 50% { opacity: .35; } }
</style>
