<script>
  import { invoke, listen, isTauri } from "./api.js";
  import { onMount } from "svelte";
  import Section from "./Section.svelte";
  import { buildConfig } from "./buildconfig.svelte.js";
  import { commandFor } from "./cli.js";

  let { onFinished } = $props();

  let java = $state(null);
  let javaError = $state("");
  let downloading = $state(null);

  let settings = $state(null);
  let areas = $state([]);
  let jarDefault = $state("");
  let jarUrl = $state("");
  let fetching = $state(null);

  async function fetchJar() {
    fetching = 0;
    const off = await listen("jar-download", (e) => {
      const { done, total } = e.payload;
      fetching = total ? Math.round((done * 100) / total) : 0;
    });
    try {
      jar = await invoke("download_planetiler");
      jarDefault = jar;
    } catch (err) {
      lines = [...lines, `ERROR: ${err}`];
    } finally {
      off();
      fetching = null;
    }
  }
  let steps = $state([]);
  let selected = $state(new Set());
  let planned = $state([]);
  let area = $state("");
  let schemaMode = $state("bundled");
  let schemaYaml = $state("");
  let jar = $state("");

  let optionDefs = $state({});
  let values = $state({});
  let presets = $state([]);
  let defaultPreset = $state("measured");
  let presetName = $state({});
  /// Free-text arguments per step, for the flags this app has no form for.
  let extraArgs = $state({});

  let running = $state(false);
  let phase = $state("");
  let label = $state("");
  let percent = $state(0);
  let lines = $state([]);
  let results = $state([]);
  /** Per-step live state, keyed by step id: what is running, what finished, how it went. */
  let status = $state({});
  let runningStep = $state(null);
  /** What is on disk, per step: the app never decides "already built" from a record alone. */
  let built = $state({});
  let force = $state(new Set());
  let forceAll = $state(false);

  onMount(async () => {
    await detect();
    try {
      settings = await invoke("get_settings");
      const defaults = await invoke("resolved_defaults");
      jarDefault = defaults.planetiler_jar ?? "";
      jarUrl = settings.planetiler_jar_url ?? "";
      // the jar the app already found beats making someone paste the same path
      jar = settings.planetiler_jar ?? "";
      // areas come from the output root, not just the config: a half-finished build is in the
      // output root and nowhere else, which is exactly when this view is needed
      areas = defaults.areas ?? [];
      area = areas[0] ?? settings.areas?.[0]?.name ?? "";
      steps = await invoke("list_steps", { area });
      presets = await invoke("list_presets");
      defaultPreset = await invoke("default_preset_name");
      for (const s of steps) {
        optionDefs[s.id] = await invoke("step_options", { step: s.id });
        // Seed from the default preset rather than leaving the form blank. cairn is here to
        // rebuild this repository's tiles, so the fields should show the values that will
        // actually be used - a blank form that silently builds something else is a trap.
        const seed = presets.find((p) => p.step === s.id && p.name === defaultPreset);
        values[s.id] = seed ? { ...seed.values } : {};
      }
      selected = new Set(["basemap"]);
      await replan();
      await refreshBuilt();
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

  async function refreshBuilt() {
    if (!area) return;
    try { built = await invoke("build_state", { area, values }); }
    catch { built = {}; }
  }

  /** Delete what a step produced. That, not a record, is what makes it run again. */
  async function clearOutputs(step) {
    confirming = "";
    clearTimeout(disarmTimer);
    const outputs = (built[step]?.outputs ?? []).map((f) => f.name);
    const what = outputs.length ? outputs.join(", ") : labelFor(step);
    if (!confirm(`Delete ${what}?`)) return;
    try {
      await invoke("clear_build_state", { area, step, deleteOutputs: true });
      await refreshBuilt();
      onFinished?.();
    } catch (err) {
      javaError = String(err);
    }
  }

  function toggleForce(step) {
    const next = new Set(force);
    next.has(step) ? next.delete(step) : next.add(step);
    force = next;
  }

  const fmtSize = (b) => (b > 1048576 ? `${(b / 1048576).toFixed(1)} MB` : `${(b / 1024).toFixed(0)} KB`);
  const fmtWhen = (secs) => (secs ? new Date(secs * 1000).toLocaleString() : "");

  async function replan() {
    try { planned = await invoke("plan_steps", { steps: [...selected] }); }
    catch { planned = [...selected]; }
  }

  async function toggle(id) {
    const next = new Set(selected);
    next.has(id) ? next.delete(id) : next.add(id);
    selected = next;
    await replan();
  }

  function applyPreset(preset) {
    values = { ...values, [preset.step]: { ...preset.values } };
  }

  async function savePreset(step) {
    const name = (presetName[step] ?? "").trim();
    if (!name || !step) return;
    await invoke("save_preset", {
      preset: { name, step, description: "", values: values[step] ?? {} },
    });
    presets = await invoke("list_presets");
    presetName = { ...presetName, [step]: "" };
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

  /// Deleting is one tap away from destroying an hour of build, so the first tap only arms it.
  /// A second tap on the armed button deletes; anything else disarms.
  let confirming = $state("");
  let disarmTimer = null;
  function arm(id) {
    confirming = id;
    clearTimeout(disarmTimer);
    disarmTimer = setTimeout(() => (confirming = ""), 4000);
  }

  async function reveal(path) {
    try { await invoke("reveal", { path }); }
    catch (err) { lines = [...lines, `ERROR: ${err}`]; }
  }

  /// Steps whose description is showing. The prose comes from the backend, so the graph and
  /// the explanation of it cannot disagree.
  let explained = $state(new Set());
  function explain(id) {
    const next = new Set(explained);
    next.has(id) ? next.delete(id) : next.add(id);
    explained = next;
  }

  let copiedLine = $state("");
  async function copy(text) {
    try {
      await navigator.clipboard.writeText(text);
      copiedLine = text;
      setTimeout(() => (copiedLine = ""), 1200);
    } catch {}
  }

  let logCopied = $state(false);
  async function copyLog() {
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      logCopied = true;
      setTimeout(() => (logCopied = false), 1200);
    } catch {}
  }

  let summaryLine = $derived.by(() => {
    if (!results.length) return "";
    const bad = results.filter((r) => !r.ok).length;
    return bad ? `${bad} of ${results.length} failed` : `${results.length} finished`;
  });

  function mark(step, patch) {
    status = { ...status, [step]: { ...(status[step] ?? {}), ...patch } };
  }

  function onEvent(ev) {
    switch (ev.event) {
      case "started":
        running = true; phase = "starting"; runningStep = ev.step;
        mark(ev.step, { state: "running", percent: 0, phase: "starting" });
        break;
      case "phase":
        phase = ev.name;
        mark(ev.step, { phase: ev.name });
        break;
      case "progress":
        label = ev.label; percent = ev.percent;
        mark(ev.step, { percent: ev.percent, label: ev.label });
        break;
      case "log": lines = [...lines.slice(-400), ev.line]; break;
      case "finished":
        results = [...results, ev];
        runningStep = null;
        mark(ev.step, { state: ev.ok ? "done" : "failed", elapsed: ev.elapsed, percent: 100 });
        refreshBuilt();
        break;
      case "skipped":
        mark(ev.step, { state: "skipped", reason: ev.reason });
        break;
    }
  }

  async function run() {
    running = true; lines = []; results = []; percent = 0;
    // queued up front, so the list reads as a plan rather than filling in as it goes
    status = Object.fromEntries(planned.map((id) => [id, { state: "queued" }]));
    try {
      await invoke("run_steps", {
        req: {
          area, steps: [...selected], values, extraArgs,
          schemaYaml: schemaMode === "yaml" ? schemaYaml : null,
          jar: jar || null,
          force: [...force],
          forceAll,
        },
      });
    } catch (err) {
      lines = [...lines, `ERROR: ${err}`];
    } finally {
      running = false;
      await refreshBuilt();
      onFinished?.();
    }
  }

  function groupsFor(step) {
    const by = new Map();
    for (const d of optionDefs[step] ?? []) {
      if (!by.has(d.group)) by.set(d.group, []);
      by.get(d.group).push(d);
    }
    return [...by.entries()];
  }
  /// Steps that take arbitrary tool arguments, and where their documentation lives. Mirroring
  /// planetiler's whole flag list here would be wrong by its next release; passing them through
  /// and pointing at the real reference stays right.
  const PASSTHROUGH = {
    basemap: {
      tool: "planetiler",
      docs: "https://github.com/onthegomap/planetiler/blob/main/PLANET.md",
      placeholder: "--max-point-buffer=4 --mlt-shared-dict",
    },
    routes: {
      tool: "planetiler",
      docs: "https://github.com/onthegomap/planetiler/blob/main/PLANET.md",
      placeholder: "--max-point-buffer=4",
    },
  };

  const labelFor = (id) => steps.find((s) => s.id === id)?.label ?? id;
  const setCountFor = (step) => Object.keys(values[step] ?? {}).length;

  // options for everything that will actually run, dependencies included - selecting two steps
  // used to leave only the last-clicked one configurable
  let optionSteps = $derived(planned.length ? planned : [...selected]);

  // the CLI view shows this same run as a command line; it reads what the form holds rather
  // than being told separately, so the two cannot describe different builds
  $effect(() => {
    buildConfig.area = area;
    buildConfig.steps = optionSteps;
    buildConfig.values = values;
    buildConfig.defs = optionDefs;
    buildConfig.extra = extraArgs;
  });
  let ready = $derived(java && (jar || jarDefault) && area && selected.size && !running);
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
    <label>Area
      {#if areas.length}
        <select value={area} onchange={(e) => { area = e.target.value; refreshBuilt(); }}>
          {#each areas as a}<option value={a}>{a}</option>{/each}
          {#if !areas.includes(area)}<option value={area}>{area}</option>{/if}
        </select>
      {:else}
        <input bind:value={area} placeholder="rhone-alpes" onchange={refreshBuilt} />
      {/if}
    </label>
    <label>Planetiler jar
      <input bind:value={jar} placeholder={jarDefault || "none found"} />
      {#if !jarDefault && !jar}
        <span class="hint">
          {#if jarUrl}
            <button class="ghost tiny" onclick={fetchJar} disabled={!!fetching}>
              {fetching ? `downloading ${fetching}%` : "Download it"}
            </button>
            from <code>{jarUrl}</code>
          {:else}
            Nothing to run builds with. Point Settings at a jar, or set a URL there to fetch one -
            it has to be a build of this pipeline's planetiler fork.
          {/if}
        </span>
      {/if}
    </label>
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
  <ul class="steplist">
    {#each steps as s}
      {@const st = status[s.id] ?? {}}
      {@const auto = !selected.has(s.id) && planned.includes(s.id)}
      {@const disk = built[s.id]}
      <li class="steprow" class:on={selected.has(s.id)} class:auto>
        <button class="pick" onclick={() => toggle(s.id)} disabled={running}
                title="include this step">
          <span class="box" class:checked={selected.has(s.id)} class:auto>
            {#if selected.has(s.id)}✓{:else if auto}+{/if}
          </span>
          <span class="sname">{s.label}</span>
        </button>

        {#if auto}<span class="tag soft">dependency</span>{/if}

        {#if st.state === "running"}
          <span class="stat run">{st.phase ?? "running"} {st.percent ?? 0}%</span>
        {:else if st.state === "done"}
          <span class="stat ok">just built{#if st.elapsed} · {st.elapsed}{/if}</span>
        {:else if st.state === "failed"}
          <span class="stat bad">failed</span>
        {:else if st.state === "skipped"}
          <span class="stat">skipped · {st.reason}</span>
        {:else if st.state === "queued"}
          <span class="stat">queued</span>
        {:else if disk?.state === "built"}
          <span class="stat ok" title={`${disk.outputs.map((f) => f.name).join(", ")}\n${fmtWhen(disk.finished_at)}`}>
            built · {disk.outputs.map((f) => (f.dir ? "directory" : fmtSize(f.bytes))).join(" + ")}
          </span>
        {:else if disk?.state === "options_changed"}
          <span class="stat warnish" title={`changed: ${disk.changed.join(", ")}`}>
            options changed
          </span>
        {:else if disk?.state === "missing"}
          <span class="stat">not built</span>
        {/if}

        {#if disk?.state === "built" || disk?.state === "options_changed"}
          {@const files = (disk.outputs ?? []).filter((f) => !f.dir)}
          <button class="mini" class:on={force.has(s.id)} disabled={running}
                  title="run this step even though its output is there"
                  onclick={() => toggleForce(s.id)}>force</button>
          {#if (s.writes ?? []).length}
            <button class="mini" title={`show ${s.writes[0]} in the file manager`}
                    onclick={() => reveal(s.writes[0])}>show</button>
          {/if}
          {#if files.length}
            {#if confirming === s.id}
              <button class="mini danger armed" disabled={running}
                      title={`delete ${files.map((f) => f.name).join(", ")}`}
                      onclick={() => clearOutputs(s.id)}>delete {fmtSize(files.reduce((n, f) => n + f.bytes, 0))}?</button>
              <button class="mini" onclick={() => (confirming = "")}>cancel</button>
            {:else}
              <button class="mini danger" disabled={running}
                      title="delete the output, so the step runs again"
                      onclick={() => arm(s.id)}>delete</button>
            {/if}
          {/if}
        {/if}

        <button class="mini info" class:on={explained.has(s.id)}
                title="what this step does" aria-label="what this step does"
                onclick={() => explain(s.id)}>?</button>

        {#if st.state === "running"}
          <div class="bar"><div class="fill" style="width:{st.percent ?? 0}%"></div></div>
        {/if}
      </li>

      {#if explained.has(s.id)}
        <li class="about">
          <p>{s.summary}</p>
          <dl>
            <dt>Needs</dt><dd>{s.reads}</dd>
            {#if s.deps?.length}
              <dt>After</dt><dd>{s.deps.map(labelFor).join(", ")}</dd>
            {/if}
            {#if s.writes?.length}
              <dt>Writes</dt>
              <dd>{#each s.writes as w}<code>{w}</code>{/each}</dd>
            {/if}
            <dt>Terminal</dt>
            <dd><code>alpimaps {s.command} --area {area || "<area>"}</code></dd>
          </dl>
        </li>
      {/if}
    {/each}
  </ul>

  <div class="runbar">
    <label class="forceall" title="ignore what is on disk and rebuild the whole plan">
      <input type="checkbox" bind:checked={forceAll} disabled={running} /> force all
    </label>
    <button onclick={run} disabled={!ready}>
      {running ? "Running…" : `Run ${planned.length || ""}`}
    </button>
    <button class="ghost" onclick={() => invoke("cancel_run")} disabled={!running}>Cancel</button>
    {#if planned.length}
      <span class="plan">{planned.map(labelFor).join(" → ")}</span>
    {/if}
  </div>
</Section>

{#each optionSteps as step, i}
  <Section title={`3.${i + 1} · ${labelFor(step)}`} open={false}
           subtitle={setCountFor(step) ? `${setCountFor(step)} set` : "defaults"}>
    {#if commandFor(step, area, values[step] ?? {}, optionDefs[step] ?? [], extraArgs[step])}
      {@const line = commandFor(step, area, values[step] ?? {}, optionDefs[step] ?? [], extraArgs[step])}
      <div class="asline">
        <code>{line}</code>
        <button class="ghost tiny" onclick={() => copy(line)}>
          {copiedLine === line ? "copied" : "copy"}
        </button>
      </div>
    {/if}

    {#if PASSTHROUGH[step]}
      <label class="extra">
        Extra {PASSTHROUGH[step].tool} arguments
        <input value={extraArgs[step] ?? ""} spellcheck="false"
               placeholder={PASSTHROUGH[step].placeholder}
               oninput={(e) => (extraArgs = { ...extraArgs, [step]: e.target.value })} />
        <span class="hint">
          Passed through verbatim, for the flags above do not cover.
          <a href={PASSTHROUGH[step].docs} target="_blank" rel="noreferrer">{PASSTHROUGH[step].tool} documentation</a>
        </span>
      </label>
    {/if}

    <div class="presets">
      {#each presets.filter((p) => p.step === step) as p}
        <button class="ghost" title={p.description} onclick={() => applyPreset(p)}>{p.name}</button>
      {/each}
      <input value={presetName[step] ?? ""} placeholder="save current as…"
             oninput={(e) => (presetName = { ...presetName, [step]: e.target.value })} />
      <button class="ghost" onclick={() => savePreset(step)}
              disabled={!(presetName[step] ?? "").trim()}>Save</button>
    </div>

    {#each groupsFor(step) as [group, defs]}
      {@const groupSet = defs.filter((d) => values[step]?.[d.key] !== undefined).length}
      <details class="group" open={groupSet > 0}>
        <summary>{group}{#if groupSet}<span class="count">{groupSet}</span>{/if}</summary>
        {#each defs as d}
          {@const val = values[step]?.[d.key]}
          {@const set = val !== undefined}
          <div class="opt" class:set>
            <div class="opthead">
              <label for={`${step}-${d.key}`}>{d.label}</label>
              {#if set}<button class="clear" onclick={() => clearValue(step, d.key)}>reset</button>{/if}
            </div>
            {#if d.kind.type === "bool"}
              <input id={`${step}-${d.key}`} type="checkbox" checked={val === true}
                     onchange={(e) => setValue(step, d.key, e.target.checked, "bool")} />
            {:else if d.kind.type === "choice"}
              <select id={`${step}-${d.key}`} value={val ?? ""}
                      onchange={(e) => setValue(step, d.key, e.target.value, "choice")}>
                <option value="">— unset —</option>
                {#each d.kind.choices as c}<option value={c}>{c}</option>{/each}
              </select>
            {:else}
              <input id={`${step}-${d.key}`}
                     type={d.kind.type === "text" ? "text" : "number"}
                     step={d.kind.type === "float" ? "0.05" : "1"}
                     value={val ?? ""}
                     oninput={(e) => setValue(step, d.key, e.target.value, d.kind.type)} />
            {/if}
            <p class="help">{d.help}</p>
            <p class="hint">unset → {d.hint}</p>
          </div>
        {/each}
      </details>
    {/each}
  </Section>
{/each}

{#if running || lines.length || results.length}
  <Section title="4 · Progress"
           subtitle={running ? `${labelFor(runningStep) || ""} · ${phase}` : summaryLine}>
    <div class="prog">
      <div class="bar big"><div class="fill" style="width:{percent}%"></div></div>
      <span class="pct">{percent}%</span>
    </div>
    <p class="muted small">{label || phase}</p>
    <details class="group" open={results.some((r) => !r.ok)}>
      <summary>
        Log
        <button class="ghost tiny" onclick={(e) => { e.preventDefault(); copyLog(); }}>
          {logCopied ? "copied" : "copy"}
        </button>
      </summary>
      <pre>{lines.slice(-150).join("\n")}</pre>
    </details>
  </Section>
{/if}

<style>
  .group { border-top: 1px solid var(--line); }
  .group summary { cursor: pointer; padding: 8px 0; font-size: 11px; text-transform: uppercase;
                   letter-spacing: .05em; color: var(--muted); list-style: none; display: flex;
                   align-items: center; gap: 8px; }
  .group summary::-webkit-details-marker { display: none; }
  .group summary::before { content: "›"; color: var(--faint); display: inline-block; }
  .group[open] summary::before { transform: rotate(90deg); }
  .count { background: var(--accent); color: var(--accent-fg); font-size: 10px; padding: 0 6px;
           border-radius: 8px; }
  label { display: block; color: var(--text-2); font-size: 13px; }
  input, select { display: block; width: 100%; margin-top: 4px; padding: 6px 9px; background: var(--bg);
          border: 1px solid var(--border); border-radius: 5px; color: var(--text); font: inherit;
          font-size: 13px; box-sizing: border-box; }
  input[type="checkbox"] { width: auto; }
  .pair { display: flex; gap: 8px; margin-top: 10px; }
  .pair label { flex: 1; }
  .steplist { list-style: none; margin: 0 0 12px; padding: 0; display: flex;
              flex-direction: column; gap: 2px; }
  .steprow { display: flex; align-items: center; gap: 8px; padding: 6px 8px;
             border-radius: var(--r); position: relative; }
  .steprow:hover { background: var(--surface-2); }
  .about { padding: 2px 10px 10px 34px; }
  .about p { color: var(--text-2); font-size: 12.5px; line-height: 1.55; margin: 0 0 8px;
             max-width: 78ch; }
  .about dl { display: grid; grid-template-columns: 74px 1fr; gap: 3px 10px; margin: 0;
              font-size: 12px; }
  .about dt { color: var(--faint); text-transform: uppercase; font-size: 10px;
              letter-spacing: .06em; padding-top: 2px; }
  .about dd { margin: 0; color: var(--text-2); }
  .about code { display: block; color: var(--text-3); font-size: 11px; }
  .info { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
  .steprow.on { background: var(--surface-2); }
  .pick { display: flex; align-items: center; gap: 9px; background: none; border: 0; padding: 0;
          color: var(--text-2); font: inherit; font-size: 13px; cursor: pointer; flex: 1;
          text-align: left; }
  .steprow.on .pick { color: var(--text); }
  .pick:disabled { cursor: not-allowed; }
  .box { width: 16px; height: 16px; flex: none; border-radius: var(--r-sm);
         border: 1px solid var(--border); display: grid; place-items: center; font-size: 11px;
         color: transparent; }
  .box.checked { background: var(--accent); border-color: var(--accent); color: #fff; }
  .box.auto { border-style: dashed; color: var(--faint); }
  .stat { font-size: 11px; color: var(--muted-2); font-variant-numeric: tabular-nums; }
  .stat.run { color: var(--ok); }
  .stat.ok { color: var(--ok); }
  .stat.bad { color: var(--danger); }
  .mini.danger { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 45%, transparent); }
  .mini.danger:hover:not(:disabled) { background: color-mix(in srgb, var(--danger) 18%, transparent);
                                      color: var(--danger); }
  /* armed: the destructive state looks destructive, and says what it will destroy */
  .mini.danger.armed { background: var(--danger); border-color: var(--danger); color: #fff; }
  .mini.danger.armed:hover:not(:disabled) { background: var(--danger); color: #fff; }
  .stat.warnish { color: var(--warn); }
  .mini { background: var(--line-2); color: var(--muted-2); font-size: 10px; padding: 2px 7px;
          border-radius: var(--r-sm); border: 1px solid transparent; }
  .mini:hover:not(:disabled) { background: var(--border); color: var(--text); }
  .mini.on { background: var(--accent); color: #fff; }
  .forceall { display: flex; align-items: center; gap: 5px; font-size: 11px; color: var(--muted-2);
              white-space: nowrap; }
  .forceall input { width: auto; margin: 0; }
  .bar { position: absolute; left: 0; right: 0; bottom: 0; height: 2px; background: var(--line-2);
         border-radius: 2px; overflow: hidden; }
  .fill { height: 100%; background: var(--accent-hi); transition: width .2s ease; }
  .runbar { display: flex; gap: 8px; align-items: center; }
  .plan { font-size: 11px; color: var(--faint); overflow: hidden; text-overflow: ellipsis;
          white-space: nowrap; }
  .tag.soft { background: var(--line-2); color: var(--muted-2); }
  .extra { display: block; margin-bottom: 10px; }
  .extra input { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
  .extra .hint { display: block; font-style: normal; margin-top: 4px; }
  .extra a { color: var(--ok); }
  .asline { display: flex; align-items: center; gap: 8px; background: var(--bg);
            border: 1px solid var(--line-2); border-radius: var(--r); padding: 7px 9px;
            margin-bottom: 10px; }
  .asline code { flex: 1; color: var(--text-3); font-size: 11px; overflow-x: auto;
                 white-space: nowrap; }
  .presets { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; margin-bottom: 8px; }
  .presets input { width: 160px; margin: 0; }
  .opt { padding: 8px 10px; border-left: 2px solid var(--line-2); margin-bottom: 8px; }
  .opt.set { border-left-color: var(--accent); background: var(--surface-2); }
  .opthead { display: flex; justify-content: space-between; align-items: baseline; }
  .clear { background: none; color: var(--muted-2); font-size: 11px; padding: 0; }
  .help { color: var(--muted); font-size: 12px; margin: 6px 0 0; }
  .hint { color: var(--faint); font-size: 11px; margin: 2px 0 0; font-style: italic; }
  .tag { background: #3a3020; color: var(--warn); font-size: 10px; padding: 1px 5px;
         border-radius: 3px; margin-left: 6px; }
  .prog { display: flex; align-items: center; gap: 10px; }
  .prog .bar { position: static; flex: 1; height: 6px; }
  .pct { font-size: 12px; color: var(--muted-2); font-variant-numeric: tabular-nums; width: 38px;
         text-align: right; }
  .small { font-size: 12px; }
  .tiny { padding: 1px 7px; font-size: 10px; margin-left: auto; }
  .muted { color: var(--muted-2); margin: 6px 0 0; }
  .ok { color: var(--ok); }
  .warn { color: var(--warn); }
  pre { margin: 8px 0 0; max-height: 220px; overflow: auto; font-size: 12px; color: var(--text-2);
        white-space: pre-wrap; word-break: break-all; }
</style>
