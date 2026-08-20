<script>
  import Areas from "./lib/Areas.svelte";
  import MapView from "./lib/MapView.svelte";
  import Build from "./lib/Build.svelte";
  import Settings from "./lib/Settings.svelte";
  import Docs from "./lib/Docs.svelte";

  let tab = $state("areas");
  let areasRef = $state(null);

  const TABS = [
    ["areas", "Output"], ["map", "Map"], ["build", "Build"],
    ["settings", "Settings"], ["docs", "Docs"],
  ];

  // digits switch tabs, the way every other tool with a tab bar does - but not while someone is
  // typing a path into a field
  function onKey(event) {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const el = event.target;
    if (el instanceof HTMLElement && (el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName))) return;
    const index = "12345".indexOf(event.key);
    if (index < 0) return;
    tab = TABS[index][0];
  }
</script>

<svelte:window onkeydown={onKey} />

<main class:wide={tab === "map"}>
  <header>
    <h1><span class="mark"></span>AlpiMaps Studio</h1>
    <nav role="tablist">
      {#each TABS as [id, label], i}
        <button role="tab" aria-selected={tab === id} class:active={tab === id}
                title={`${label}  (${i + 1})`} onclick={() => (tab = id)}>
          {label}<kbd>{i + 1}</kbd>
        </button>
      {/each}
    </nav>
  </header>

  {#if tab === "areas"}
    <Areas bind:this={areasRef} />
  {:else if tab === "map"}
    <MapView />
  {:else if tab === "build"}
    <Build onFinished={() => areasRef?.refresh()} />
  {:else if tab === "settings"}
    <Settings />
  {:else}
    <Docs />
  {/if}
</main>

<style>
  /* One palette and one scale for the whole app: every component pulls from these, so a
     change lands everywhere instead of in whichever file was edited last. */
  :global(:root) {
    --bg: #12151a;
    --bg-sunken: #0f1115;
    --surface: #161b22;
    --surface-2: #171c24;
    --card: #1a1f27;
    --hover: #1f2630;
    --line: #1e242d;
    --line-2: #262d38;
    --border: #303845;
    --disabled-bg: #22282f;

    --accent: #2d7a5c;
    --accent-hi: #379469;
    --accent-dim: #1e4436;
    --accent-fg: #cfe8db;

    --ok: #7cc9a0;
    --warn: #d99a5b;
    --danger: #e6584d;

    --text: #dde3ea;
    --text-2: #9aa5b1;
    --text-3: #8a94a2;
    --muted: #7c8896;
    --muted-2: #6b7684;
    --faint: #5d6673;

    --r-sm: 4px;
    --r: 6px;
    --r-lg: 9px;
    --shadow: 0 6px 20px rgba(0, 0, 0, .35);
    --focus: 0 0 0 2px rgba(55, 148, 105, .55);
  }
  :global(body) {
    margin: 0;
    font: 14px/1.5 ui-sans-serif, system-ui, sans-serif;
    background: var(--bg);
    color: var(--text);
  }
  :global(button) {
    padding: 7px 13px; background: var(--accent); border: 0; border-radius: var(--r);
    color: #fff; font: inherit; cursor: pointer;
    transition: background .12s ease, color .12s ease, opacity .12s ease;
  }
  :global(button:hover:not(:disabled)) { background: var(--accent-hi); }
  :global(button.ghost) { background: var(--line-2); color: var(--text-2); }
  :global(button.ghost:hover:not(:disabled)) { background: var(--border); color: var(--text); }
  :global(button:disabled) { background: var(--disabled-bg); color: var(--faint); cursor: not-allowed; }
  /* one focus treatment everywhere, and only for keyboard users */
  :global(:focus-visible) { outline: none; box-shadow: var(--focus); border-radius: var(--r-sm); }
  :global(code) { font-size: 12px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
  :global(::-webkit-scrollbar) { width: 10px; height: 10px; }
  :global(::-webkit-scrollbar-thumb) { background: var(--line-2); border-radius: 6px;
                                       border: 2px solid var(--bg); }
  :global(::-webkit-scrollbar-thumb:hover) { background: var(--border); }
  :global(::-webkit-scrollbar-track) { background: transparent; }
  main { max-width: 1100px; margin: 0 auto; padding: 20px 20px 60px; }
  /* the map wants the whole window; every other tab reads better in a column */
  main.wide { max-width: none; height: 100vh; padding: 16px 16px 12px; box-sizing: border-box;
              display: flex; flex-direction: column; }
  main.wide header { flex: none; }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 14px;
           padding-bottom: 12px; border-bottom: 1px solid var(--line-2); }
  h1 { font-size: 15px; font-weight: 600; margin: 0; letter-spacing: -.01em;
       display: flex; align-items: center; gap: 9px; }
  .mark { width: 9px; height: 9px; border-radius: 2px; background: var(--accent-hi);
          box-shadow: 0 0 0 3px var(--accent-dim); }
  nav { display: flex; gap: 2px; background: var(--surface); border: 1px solid var(--line-2);
        border-radius: var(--r-lg); padding: 3px; }
  nav button { background: transparent; color: var(--muted); padding: 5px 12px;
               border-radius: var(--r); font-size: 13px; display: flex; align-items: center; gap: 6px; }
  nav button:hover:not(.active) { background: var(--hover); color: var(--text-2); }
  nav button.active { background: var(--card); color: var(--text); box-shadow: var(--shadow); }
  kbd { font: inherit; font-size: 10px; color: var(--faint); background: var(--bg);
        border: 1px solid var(--line-2); border-radius: 3px; padding: 0 4px; line-height: 1.4; }
  nav button.active kbd { color: var(--muted); }
</style>
