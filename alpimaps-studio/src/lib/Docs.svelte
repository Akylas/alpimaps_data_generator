<script>
  // What the app does and how to drive it from a terminal.
  //
  // The step list is read from the backend rather than written out here: a step added to the
  // graph appears in the docs on its own, instead of the docs quietly going out of date.
  import { invoke } from "./api.js";
  import { onMount } from "svelte";
  import Section from "./Section.svelte";

  let steps = $state([]);
  let paths = $state(null);

  onMount(async () => {
    try {
      steps = await invoke("list_steps");
      paths = await invoke("get_settings");
    } catch {
      // docs are readable without a backend; the generated bits just stay empty
    }
  });

  /// What each step reads and writes, in the words someone debugging a build would want.
  const STEP_NOTES = {
    download_osm: {
      cli: "alpimaps download --area <area>",
      writes: "data/sources/<area_with_underscores>.osm.pbf",
      note: "Resolves the extract through Geofabrik's index, so the area id is theirs (`rhone-alpes`, `france`). One copy feeds the basemap, the routes and the Valhalla graph.",
    },
    elevation_tiles: {
      cli: "alpimaps elevation --area <area>",
      writes: "elevation_tiles/ (shared between areas)",
      note: "Runs valhalla_build_elevation with -d, so the .hgt land decompressed - the terrain step reads the same files later.",
    },
    basemap: {
      cli: "alpimaps basemap --area <area>",
      writes: "<area>/<area>.mbtiles",
      note: "Planetiler with the bundled OpenMapTiles fork, or a YAML schema. Options are the ones under 3 · Basemap; -o key=value from the CLI.",
    },
    routes: {
      cli: "alpimaps routes --area <area>",
      writes: "<area>/<area>_routes.mbtiles",
      note: "The same planetiler run restricted to the route layer. Hiking and cycling relations only, which is why it is a fraction of the basemap's size.",
    },
    terrain_rgb: {
      cli: "alpimaps terrain --area <area>",
      writes: "<area>/<area>_terrain.mbtiles",
      note: "Terrarium-packed elevation from the sources in sources.json, lowest priority first. The map view draws hillshade from these; there is no contour archive any more.",
    },
    hillshade: {
      cli: "alpimaps terrain --area <area> -o encoding=mapbox",
      writes: "<area>/<area>_hillshade.mbtiles",
      note: "The same pyramid packed the mapbox way. It exists because older archives are named this and the app still reads them.",
    },
    valhalla_tiles: {
      cli: "alpimaps valhalla-tiles --area <area>",
      writes: "valhalla_tiles/ (shared between areas)",
      note: "valhalla_build_tiles over the OSM extract, using the configured valhalla.json. Slow, and worth building once for a parent area covering everything you route in.",
    },
    valhalla_package: {
      cli: "alpimaps package --area <area>",
      writes: "<area>/<area>.vtiles",
      note: "Packs the graph tiles for one area into the archive the phone downloads. Takes its tile list from an existing package.",
    },
  };

  const CLI = [
    ["alpimaps catalog", "List areas and artifacts, with sizes and zoom ranges."],
    ["alpimaps state --area <area>", "What is already built, from the files on disk."],
    ["alpimaps state --area <area> clear [step]", "Delete a step's output so it runs again."],
    ["alpimaps state --area <area> forget [step]", "Drop the recorded options; output stays."],
    ["alpimaps options <step>", "Every option a step accepts, with its default."],
    ["alpimaps route --tiles <pkg> --point lon,lat --point lon,lat", "Route through a package."],
    ["alpimaps profile --path <terrain.mbtiles> --point lon,lat …", "Sample elevation along a line."],
    ["alpimaps serve <output_root>", "Serve the output for a browser or this app."],
  ];

  const FLAGS = [
    ["--repo <dir>", "Repository root. Every other path defaults from it."],
    ["--output <dir>", "Output root. Defaults to <repo>/alpimaps_mbtiles."],
    ["--force", "Run even though the output is already there."],
    ["--dry-run", "Print the command that would run, and stop."],
    ["-o key=value", "Override one option; repeatable, checked against the step's schema."],
    ["--preset <name>", "Start from a saved option set, then apply any -o on top."],
  ];
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
        {@const note = STEP_NOTES[s.id] ?? {}}
        <tr>
          <td>
            <strong>{s.label}</strong>
            {#if note.note}<p class="note">{note.note}</p>{/if}
          </td>
          <td><code>{note.writes ?? "—"}</code></td>
          <td><code>{note.cli ?? "—"}</code></td>
        </tr>
      {/each}
      {#if !steps.length}
        <tr><td colspan="3" class="muted">step list unavailable</td></tr>
      {/if}
    </tbody>
  </table>
</Section>

<Section title="Map view" open={false}>
  <ul>
    <li><strong>Inspect</strong> — click a feature to read its properties.</li>
    <li><strong>Route</strong> — needs a <code>.vtiles</code> package and a build with Valhalla linked; the picker chooses which package to route on.</li>
    <li><strong>Profile</strong> — samples elevation from the area's terrain archive.</li>
    <li><strong>Tiles</strong> — dumps the clicked tile's contents as JSON, with a copy button.</li>
    <li><strong>Style</strong> — points a MapLibre style at an archive and re-applies it as you edit. Leaving the mode restores the real style.</li>
  </ul>
  <p class="muted">
    Terrain archives can be drawn as hillshade, as the raw encoded bytes, or as 3D terrain -
    which is the one that shows tile edges, because a mismatched edge becomes a cliff. Grid
    draws tile boundaries with their z/x/y.
  </p>
</Section>

<Section title="Command line" open={false}>
  <table>
    <tbody>
      {#each CLI as [cmd, what]}
        <tr><td><code>{cmd}</code></td><td class="muted">{what}</td></tr>
      {/each}
    </tbody>
  </table>
  <h4>Flags worth knowing</h4>
  <table>
    <tbody>
      {#each FLAGS as [flag, what]}
        <tr><td><code>{flag}</code></td><td class="muted">{what}</td></tr>
      {/each}
    </tbody>
  </table>
</Section>

<Section title="Where things live" open={false}>
  {#if paths}
    <table>
      <tbody>
        <tr><td>Repo root</td><td><code>{paths.repo_root}</code></td></tr>
        <tr><td>Output root</td><td><code>{paths.output_root}</code></td></tr>
        <tr><td>OSM downloads</td><td><code>{paths.data_dir}</code></td></tr>
        <tr><td>Elevation tiles</td><td><code>{paths.elevation_tiles_dir}</code></td></tr>
        <tr><td>Elevation sources</td><td><code>{paths.sources_json}</code></td></tr>
        <tr><td>Valhalla binaries</td><td><code>{paths.valhalla_bin_dir ?? "not set"}</code></td></tr>
        <tr><td>valhalla.json</td><td><code>{paths.valhalla_config ?? "<repo>/valhalla.json"}</code></td></tr>
      </tbody>
    </table>
    <p class="muted">All of these are editable in Settings.</p>
  {:else}
    <p class="muted">paths unavailable outside the app</p>
  {/if}
</Section>

<style>
  p { margin: 0 0 8px; line-height: 1.55; }
  .muted { color: var(--muted-2); }
  .note { color: var(--muted-2); font-size: 12px; margin: 4px 0 0; }
  ul { margin: 0; padding-left: 18px; color: var(--text-2); }
  li { margin-bottom: 5px; line-height: 1.5; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th { text-align: left; color: var(--muted-2); font-weight: 500; font-size: 11px;
       text-transform: uppercase; letter-spacing: .05em; padding: 6px 8px;
       border-bottom: 1px solid var(--line-2); }
  td { padding: 8px; border-bottom: 1px solid var(--line); vertical-align: top; }
  /* break at spaces, not mid-flag: `--ar ea` is worse than a slightly wider column */
  td code { color: var(--text-3); overflow-wrap: break-word; }
  h4 { font-size: 11px; text-transform: uppercase; letter-spacing: .06em; color: var(--muted-2);
       margin: 16px 0 6px; }
</style>
