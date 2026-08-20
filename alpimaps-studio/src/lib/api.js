// One entry point for backend calls, so the UI runs in two places.
//
// Inside the Tauri webview it forwards to the real commands. In a plain browser (`npm run dev`)
// it falls back to the standalone tile server, which exposes the same catalog, tile and profile
// endpoints. That fallback is what makes the map testable without building the whole app - and
// how the blank-map bug below was actually found.
const inTauri = typeof window !== "undefined" && !!window.__TAURI_INTERNALS__;

/** Dev-only: where `cargo run -p studio-core --example serve` is listening. */
const DEV_BASE =
  new URLSearchParams(location.search).get("tiles") ||
  localStorage.getItem("tilesBase") ||
  "http://127.0.0.1:8787";

async function devInvoke(cmd, args) {
  switch (cmd) {
    case "start_tiles":
      return DEV_BASE;
    case "list_areas": {
      const res = await fetch(`${DEV_BASE}/catalog`);
      if (!res.ok) throw new Error(`catalog ${res.status}`);
      return res.json();
    }
    case "list_steps": {
      const url = new URL(`${DEV_BASE}/steps`);
      if (args?.area) url.searchParams.set("area", args.area);
      return (await fetch(url)).json();
    }
    case "step_options":
      return (await fetch(`${DEV_BASE}/step-options/${args.step}`)).json();
    case "reveal":
      throw new Error("the file manager is only reachable from the app");
    case "cli_reference":
      // the browser cannot run a binary; the app reads the real --help
      return { path: null, usage: "", commands: [], hint: "available inside the app" };
    case "resolved_defaults": {
      const areas = await (await fetch(`${DEV_BASE}/catalog`)).json();
      return { planetiler_jar: null, valhalla_config: null, areas: areas.map((a) => a.name) };
    }
    case "build_state":
      return (await fetch(`${DEV_BASE}/build-state/${args.area}`)).json();
    case "list_presets":
      return (await fetch(`${DEV_BASE}/presets`)).json();
    case "plan_steps": {
      const res = await fetch(`${DEV_BASE}/plan`, {
        method: "POST", headers: { "content-type": "application/json" },
        body: JSON.stringify(args.steps),
      });
      return res.json();
    }
    case "routing_status":
      return (await fetch(`${DEV_BASE}/routing-status`)).json();
    case "valhalla_route": {
      const res = await fetch(`${DEV_BASE}/route`, {
        method: "POST", headers: { "content-type": "application/json" },
        body: JSON.stringify({
          locations: args.req.locations,
          costing: args.req.costing,
          package: args.req.package,
        }),
      });
      if (!res.ok) throw new Error(await res.text());
      return res.text();
    }
    case "get_settings":
      // dev mode has no settings store; enough shape for the form to render
      return { areas: [{ name: "rhone-alpes" }], planetiler_jar: "", heap_mb: 12288 };
    case "elevation_profile": {
      const r = args.req;
      const res = await fetch(`${DEV_BASE}/profile`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          path: r.path, line: r.line, zoom: r.zoom,
          densify_m: r.densifyM, threshold_m: r.thresholdM,
        }),
      });
      if (!res.ok) throw new Error(await res.text());
      return res.json();
    }
    default:
      throw new Error(`${cmd} is not available in browser dev mode`);
  }
}

export async function invoke(cmd, args) {
  if (!inTauri) return devInvoke(cmd, args);
  const { invoke: real } = await import("@tauri-apps/api/core");
  return real(cmd, args);
}

export async function listen(event, handler) {
  if (!inTauri) return () => {};
  const { listen: real } = await import("@tauri-apps/api/event");
  return real(event, handler);
}

export const isTauri = inTauri;
