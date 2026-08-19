<script>
  import Areas from "./lib/Areas.svelte";
  import MapView from "./lib/MapView.svelte";
  import Build from "./lib/Build.svelte";
  import Settings from "./lib/Settings.svelte";

  let tab = $state("areas");
  let areasRef = $state(null);

  const TABS = [["areas", "Output"], ["map", "Map"], ["build", "Build"], ["settings", "Settings"]];
</script>

<main class:wide={tab === "map"}>
  <header>
    <h1>AlpiMaps Studio</h1>
    <nav>
      {#each TABS as [id, label]}
        <button class:active={tab === id} onclick={() => (tab = id)}>{label}</button>
      {/each}
    </nav>
  </header>

  {#if tab === "areas"}
    <Areas bind:this={areasRef} />
  {:else if tab === "map"}
    <MapView />
  {:else if tab === "build"}
    <Build onFinished={() => areasRef?.refresh()} />
  {:else}
    <Settings />
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    font: 14px/1.5 ui-sans-serif, system-ui, sans-serif;
    background: #12151a;
    color: #dde3ea;
  }
  :global(button) {
    padding: 7px 13px; background: #2d5f4a; border: 0; border-radius: 5px; color: #fff;
    font: inherit; cursor: pointer;
  }
  :global(button.ghost) { background: #262d38; color: #9aa5b1; }
  :global(button:disabled) { background: #22282f; color: #5d6673; cursor: not-allowed; }
  :global(code) { font-size: 12px; }
  main { max-width: 1100px; margin: 0 auto; padding: 20px 20px 60px; }
  /* the map wants the whole window; every other tab reads better in a column */
  main.wide { max-width: none; height: 100vh; padding: 16px 16px 12px; box-sizing: border-box;
              display: flex; flex-direction: column; }
  main.wide header { flex: none; }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 14px; }
  h1 { font-size: 17px; font-weight: 600; margin: 0; }
  nav { display: flex; gap: 4px; }
  nav button { background: transparent; color: #7c8896; padding: 6px 12px; }
  nav button.active { background: #1a1f27; color: #dde3ea; }
</style>
