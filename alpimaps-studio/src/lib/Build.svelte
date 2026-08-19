<script>
  import { invoke, listen, isTauri } from "./api.js";
  import { onMount } from "svelte";
  import Section from "./Section.svelte";

  let java = $state(null);
  let javaError = $state("");
  let downloading = $state(null);

  let settings = $state(null);
  let steps = $state([]);
  let selected = $state(new Set());
  let planned = $state([]);
  let area = $state("");
  let schemaMode = $state("bundled");
  let schemaYaml = $state("");
  let jar = $state("");

  let activeStep = $state(null);
  let optionDefs = $state({});
  let values = $state({});
  let presets = $state([]);
  let presetName = $state("");

  let running = $state(false);
  let phase = $state("");
  let label = $state("");
  let percent = $state(0);
  let lines = $state([]);
  let results = $state([]);

  onMount(async () => {
    await detect();
    try {
      settings = await invoke("get_settings");
      jar = settings.planetiler_jar ?? "";
      area = settings.areas?.[0]?.name ?? "";
      steps = await invoke("list_steps");
      presets = await invoke("list_presets");
      for (const s of steps) {
        optionDefs[s.id] = await invoke("step_options", { step: s.id });
        values[s.id] = {};
      }
      selected = new Set(["basemap"]);
      activeStep = "basemap";
      await replan();
    } catch (err) {
      javaError = String(err);
    }
    const off = await listen("step", (e) => onEvent(e.payload));
    return () => off();
  });

  async function detect() {
    javaError = "";
    try { java = await invoke("detect_java"); }
    catch (err) { java = null; javaError = String(err); }
  }

  async function downloadJre() {
    downloading = { done: 0, total: null };
    try { java = await invoke("download_java"); }
    catch (err) { javaError = String(err); }
    finally { downloading = null; }
  }

  async function replan() {
    try { planned = await invoke("plan_steps", { steps: [...selected] }); }
    catch { planned = [...selected]; }
  }

  async function toggle(id) {
    const next = new Set(selected);
    next.has(id) ? next.delete(id) : next.add(id);
    selected = next;
    // always focus the clicked step, selected or not - otherwise the only way to look at a
    // step's options is to toggle it on, and the only way to leave is to toggle it off
    activeStep = id;
    await replan();
  }

  function applyPreset(preset) {
    values = { ...values, [preset.step]: { ...preset.values } };
  }

  async function savePreset() {
    const name = presetName.trim();
    if (!name || !activeStep) return;
    await invoke("save_preset", {
      preset: { name, step: activeStep, description: "", values: values[activeStep] ?? {} },
    });
    presets = await invoke("list_presets");
    presetName = "";
  }

  function setValue(step, key, raw, kind) {
    const next = { ...(values[step] ?? {}) };
    if (raw === "" && kind !== "text") delete next[key];
    else if (kind === "bool") next[key] = raw;
    else if (kind === "int") next[key] = parseInt(raw, 10);
    else if (kind === "float") next[key] = parseFloat(raw);
    else next[key] = raw;
    values = { ...values, [step]: next };
  }

  function clearValue(step, key) {
    const next = { ...(values[step] ?? {}) };
    delete next[key];
    values = { ...values, [step]: next };
  }

  function onEvent(ev) {
    switch (ev.event) {
      case "started": running = true; phase = "starting"; break;
      case "phase": phase = ev.name; break;
      case "progress": label = ev.label; percent = ev.percent; break;
      case "log": lines = [...lines.slice(-400), ev.line]; break;
      case "finished":
        results = [...results, ev];
        break;
    }
  }

  async function run() {
    running = true; lines = []; results = []; percent = 0;
    try {
      await invoke("run_steps", {
        req: {
          area, steps: [...selected], values,
          schemaYaml: schemaMode === "yaml" ? schemaYaml : null,
          jar: jar || null,
        },
      });
    } catch (err) {
      lines = [...lines, `ERROR: ${err}`];
    } finally {
      running = false;
    }
  }

  let groups = $derived.by(() => {
    const defs = optionDefs[activeStep] ?? [];
    const by = new Map();
    for (const d of defs) {
      if (!by.has(d.group)) by.set(d.group, []);
      by.get(d.group).push(d);
    }
    return [...by.entries()];
  });
  let stepPresets = $derived(presets.filter((p) => p.step === activeStep));
  let setCount = $derived(Object.keys(values[activeStep] ?? {}).length);
  let ready = $derived(java && jar && area && selected.size && !running);
</script>

{#if !isTauri}
  <p class="warn">Browser dev mode — builds run only inside the app.</p>
{/if}

<Section title="1 · Runtime" open={!java}
         badge={java ? "ready" : "needs Java"}
         subtitle={java ? `Java ${java.version} · ${area || "no area"}` : ""}>
  {#if java}
    <p class="ok">Java {java.version} · <code>{java.source}</code></p>
  {:else if downloading}
    <p>Downloading JRE… {(downloading.done / 1048576).toFixed(1)} MB</p>
  {:else}
    <p class="warn">No Java 21+ found. {javaError}</p>
    <button onclick={downloadJre}>Download JRE 21</button>
  {/if}
  <div class="pair">
    <label>Area<input bind:value={area} placeholder="rhone-alpes" /></label>
    <label>Planetiler jar<input bind:value={jar} /></label>
  </div>
  <div class="pair">
    <label>Schema
      <select bind:value={schemaMode}>
        <option value="bundled">Bundled OpenMapTiles fork</option>
        <option value="yaml">YAML schema (custommap)</option>
      </select>
    </label>
    {#if schemaMode === "yaml"}
      <label>Schema file<input bind:value={schemaYaml} placeholder="…/shortbread.yml" /></label>
    {/if}
  </div>
</Section>

<Section title="2 · Steps" subtitle={planned.length ? `${planned.length} to run` : "nothing selected"}>
  <div class="steps">
    {#each steps as s}
      <button class="step" class:on={selected.has(s.id)} class:todo={!s.implemented}
              onclick={() => toggle(s.id)}>
        {s.label}
        {#if !s.implemented}<span class="tag">not wired</span>{/if}
      </button>
    {/each}
  </div>
  {#if planned.length}
    <p class="muted">
      Plan: {planned.map((p) => steps.find((s) => s.id === p)?.label ?? p).join(" → ")}
    </p>
  {/if}
  <div class="row">
    <button onclick={run} disabled={!ready}>Run {planned.length || ""}</button>
    <button class="ghost" onclick={() => invoke("cancel_run")} disabled={!running}>Cancel</button>
  </div>
</Section>

{#if activeStep}
  <Section title="3 · Options" open={false}
           subtitle={`${steps.find((s) => s.id === activeStep)?.label ?? ""} · ${setCount} set`}>
    <div class="presets">
      {#each stepPresets as p}
        <button class="ghost" title={p.description} onclick={() => applyPreset(p)}>{p.name}</button>
      {/each}
      <input bind:value={presetName} placeholder="save current as…" />
      <button class="ghost" onclick={savePreset} disabled={!presetName.trim()}>Save</button>
    </div>

    {#each groups as [group, defs]}
      {@const groupSet = defs.filter((d) => values[activeStep]?.[d.key] !== undefined).length}
      <details class="group" open={groupSet > 0}>
        <summary>{group}{#if groupSet}<span class="count">{groupSet}</span>{/if}</summary>
      {#each defs as d}
        {@const val = values[activeStep]?.[d.key]}
        {@const set = val !== undefined}
        <div class="opt" class:set>
          <div class="opthead">
            <label for={d.key}>{d.label}</label>
            {#if set}<button class="clear" onclick={() => clearValue(activeStep, d.key)}>reset</button>{/if}
          </div>
          {#if d.kind.type === "bool"}
            <input id={d.key} type="checkbox" checked={val === true}
                   onchange={(e) => setValue(activeStep, d.key, e.target.checked, "bool")} />
          {:else if d.kind.type === "choice"}
            <select id={d.key} value={val ?? ""}
                    onchange={(e) => setValue(activeStep, d.key, e.target.value, "choice")}>
              <option value="">— unset —</option>
              {#each d.kind.choices as c}<option value={c}>{c}</option>{/each}
            </select>
          {:else}
            <input id={d.key}
                   type={d.kind.type === "text" ? "text" : "number"}
                   step={d.kind.type === "float" ? "0.05" : "1"}
                   value={val ?? ""}
                   oninput={(e) => setValue(activeStep, d.key, e.target.value, d.kind.type)} />
          {/if}
          <p class="help">{d.help}</p>
          <p class="hint">unset → {d.hint}</p>
        </div>
      {/each}
      </details>
    {/each}
  </Section>
{/if}

{#if running || lines.length || results.length}
  <Section title="4 · Progress" subtitle={phase}>
    <p class="phase">{phase}</p>
    <progress max="100" value={percent}></progress>
    <p class="muted">{label} {percent}%</p>
    {#each results as r}
      <p class={r.ok ? "ok" : "warn"}>
        {r.step}: {r.ok ? "finished" : "failed"}{#if r.elapsed} in {r.elapsed}{/if}
      </p>
    {/each}
    <details class="group">
      <summary>Log</summary>
      <pre>{lines.slice(-150).join("\n")}</pre>
    </details>
  </Section>
{/if}

<style>
  .group { border-top: 1px solid #222932; }
  .group summary { cursor: pointer; padding: 8px 0; font-size: 11px; text-transform: uppercase;
                   letter-spacing: .05em; color: #7c8896; list-style: none; display: flex;
                   align-items: center; gap: 8px; }
  .group summary::-webkit-details-marker { display: none; }
  .group summary::before { content: "›"; color: #5d6673; display: inline-block; }
  .group[open] summary::before { transform: rotate(90deg); }
  .count { background: #2d5f4a; color: #cfe8db; font-size: 10px; padding: 0 6px;
           border-radius: 8px; }
  label { display: block; color: #9aa5b1; font-size: 13px; }
  input, select { display: block; width: 100%; margin-top: 4px; padding: 6px 9px; background: #12151a;
          border: 1px solid #303845; border-radius: 5px; color: #dde3ea; font: inherit;
          font-size: 13px; box-sizing: border-box; }
  input[type="checkbox"] { width: auto; }
  .pair { display: flex; gap: 8px; margin-top: 10px; }
  .pair label { flex: 1; }
  .row { display: flex; gap: 8px; margin-top: 12px; }
  .steps { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 10px; }
  .step { background: #12151a; border: 1px solid #303845; color: #9aa5b1; padding: 6px 12px; }
  .step.on { background: #2d5f4a; border-color: #2d5f4a; color: #fff; }
  .step.todo { opacity: .6; }
  .presets { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; margin-bottom: 8px; }
  .presets input { width: 160px; margin: 0; }
  .opt { padding: 8px 10px; border-left: 2px solid #262d38; margin-bottom: 8px; }
  .opt.set { border-left-color: #2d5f4a; background: #171c24; }
  .opthead { display: flex; justify-content: space-between; align-items: baseline; }
  .clear { background: none; color: #6b7684; font-size: 11px; padding: 0; }
  .help { color: #7c8896; font-size: 12px; margin: 6px 0 0; }
  .hint { color: #5d6673; font-size: 11px; margin: 2px 0 0; font-style: italic; }
  .tag { background: #3a3020; color: #d9a95b; font-size: 10px; padding: 1px 5px;
         border-radius: 3px; margin-left: 6px; }
  progress { width: 100%; height: 6px; }
  .phase { font-weight: 600; margin: 0 0 6px; }
  .muted { color: #6b7684; margin: 6px 0 0; }
  .ok { color: #7cc9a0; }
  .warn { color: #d99a5b; }
  pre { margin: 8px 0 0; max-height: 220px; overflow: auto; font-size: 12px; color: #98a3b0;
        white-space: pre-wrap; word-break: break-all; }
</style>
