<script>
  // What the app does and how to drive it from a terminal.
  //
  // The step list is read from the backend rather than written out here: a step added to the
  // graph appears in the docs on its own, instead of the docs quietly going out of date.
  import { invoke } from "./api.js";
  import { onMount } from "svelte";
  import Section from "./Section.svelte";
  import Cli from "./Cli.svelte";
  import { MAP_MODES, TERRAIN_MODE_HELP } from "./modes.js";

  let steps = $state([]);
  let paths = $state(null);
  let resolved = $state(null);

  const label = (id) => steps.find((s) => s.id === id)?.label ?? id;

  onMount(async () => {
    try {
      steps = await invoke("list_steps");
      paths = await invoke("get_settings");
      resolved = await invoke("resolved_defaults");
    } catch {
      // docs are readable without a backend; the generated bits just stay empty
    }
  });

</script>

<Section title="What this is" open={true}>
  <p>
    One pipeline, two front ends. The steps below run the same code whether they are started
    from the Build tab or from <code>alpimaps</code> in a terminal, so a build started in one
    place can be finished in the other.
  </p>
  <p class="muted">
    Everything an area produces lives in one directory under the output root, and that directory
    is the whole story: delete a file and the step that makes it runs again.
  </p>
</Section>

<Section title="Build state" open={true}>
  <p>
    A step counts as <strong>built</strong> when the files it produces are present and not
    empty. Nothing else. Output from the shell scripts, or copied in from another machine,
    counts immediately; a zero-byte file left by an interrupted run does not.
  </p>
  <ul>
    <li><strong>force</strong> runs one step anyway, <strong>force all</strong> the whole plan.</li>
    <li><strong>delete</strong> removes that step's output, which is what makes it run again.</li>
    <li>
      Editing an option marks a built step as <em>options changed</em>, because the record next
      to the artifacts (<code>.studio-state.json</code>) remembers what it ran with. That record
      only ever adds the timing and the option check - losing it never loses the output.
    </li>
    <li>Shared directories (elevation tiles, the Valhalla graph) are never deleted from here.</li>
  </ul>
</Section>

<Section title="Steps" open={true}>
  <table>
    <thead>
      <tr><th>Step</th><th>Writes</th><th>From a terminal</th></tr>
    </thead>
    <tbody>
      {#each steps as s}
        <tr>
          <td>
            <strong>{s.label}</strong>
            <p class="note">{s.summary}</p>
            <p class="note">
              Needs {s.reads}{#if s.deps?.length}, after {s.deps.map((d) => label(d)).join(", ")}{/if}.
            </p>
          </td>
          <td>
            {#each s.writes ?? [] as w}<code>{w}</code>{/each}
            {#if !(s.writes ?? []).length}<code>—</code>{/if}
          </td>
          <td><code>alpimaps {s.command} --area &lt;area&gt;</code></td>
        </tr>
      {/each}
      {#if !steps.length}
        <tr><td colspan="3" class="muted">step list unavailable</td></tr>
      {/if}
    </tbody>
  </table>
</Section>

<Section title="Map view" open={false}>
  <table>
    <tbody>
      {#each MAP_MODES as m}
        <tr>
          <td><strong>{m.label}</strong></td>
          <td>
            <p class="note">{m.summary}</p>
            {#if m.needs}<p class="note">Needs {m.needs}.</p>{/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
  <h4>Drawing a terrain archive</h4>
  <table>
    <tbody>
      {#each Object.entries(TERRAIN_MODE_HELP) as [mode, what]}
        <tr><td><code>{mode === "terrain3d" ? "3D" : mode}</code></td><td class="muted">{what}</td></tr>
      {/each}
    </tbody>
  </table>
  <p class="note">
    Grid draws tile boundaries with their z/x/y. Both side panels collapse, and the right one
    holds the layers of the comparison map - add layers there, then Compare swipes between them.
  </p>
</Section>

<Section title="Command line" open={false}
         subtitle="read from the binary, so it cannot drift">
  <Cli />
</Section>

<Section title="Where things live" open={false}>
  {#if paths}
    <h4>Your data</h4>
    <table>
      <tbody>
        <tr><td>Output root</td><td><code>{paths.output_root}</code></td></tr>
        <tr><td>OSM downloads</td><td><code>{paths.data_dir}</code></td></tr>
        <tr><td>Elevation tiles</td><td><code>{paths.elevation_tiles_dir}</code></td></tr>
        <tr><td>Elevation sources</td><td><code>{paths.sources_json}</code></td></tr>
        <tr><td>Scratch</td><td><code>{paths.tmp_dir}</code></td></tr>
      </tbody>
    </table>

    <h4>Tools</h4>
    <p class="note">
      Resolved in this order: what Settings names, then the copy shipped inside the app, then a
      repository checkout, then <code>PATH</code>. A packaged install has no checkout, so what it
      finds is what was bundled with it.
    </p>
    <table>
      <tbody>
        <tr>
          <td>Planetiler jar</td>
          <td><code class:missing={!resolved?.planetiler_jar}>{resolved?.planetiler_jar ?? "not found"}</code></td>
        </tr>
        <tr>
          <td>valhalla.json</td>
          <td><code class:missing={!resolved?.valhalla_config}>{resolved?.valhalla_config ?? "not found"}</code></td>
        </tr>
        {#each resolved?.valhalla_tools ?? [] as [name, found]}
          <tr>
            <td>{name}</td>
            <td><code class:missing={!found}>{found ?? "not found - that step cannot run"}</code></td>
          </tr>
        {/each}
        <tr>
          <td>Bundled files</td>
          <td><code>{resolved?.resource_dir ?? "not a packaged build"}</code></td>
        </tr>
        <tr><td>Repository</td><td><code>{paths.repo_root}</code></td></tr>
      </tbody>
    </table>
    <p class="note">All of these are editable in Settings.</p>
  {:else}
    <p class="muted">paths unavailable outside the app</p>
  {/if}
</Section>

<style>
  p { margin: 0 0 8px; line-height: 1.55; }
  .muted { color: var(--muted-2); }
  .note { color: var(--muted-2); font-size: 12px; margin: 4px 0 0; line-height: 1.5; }
  .missing { color: var(--warn); }
  h4 { font-size: 11px; text-transform: uppercase; letter-spacing: .06em; color: var(--muted-2);
       margin: 16px 0 6px; font-weight: 600; }
  ul { margin: 0; padding-left: 18px; color: var(--text-2); }
  li { margin-bottom: 5px; line-height: 1.5; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th { text-align: left; color: var(--muted-2); font-weight: 500; font-size: 11px;
       text-transform: uppercase; letter-spacing: .05em; padding: 6px 8px;
       border-bottom: 1px solid var(--line-2); }
  td { padding: 8px; border-bottom: 1px solid var(--line); vertical-align: top; }
  /* break at spaces, not mid-flag: `--ar ea` is worse than a slightly wider column */
  td code { color: var(--text-3); overflow-wrap: break-word; }
</style>
