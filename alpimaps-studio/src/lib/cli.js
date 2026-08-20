// Turning what the Build form holds into the command line that does the same thing.
//
// The flag names are not written down here: they come from the same option schema the form is
// generated from, where each option carries the flag it maps to. So a schema change moves the
// form and this together, and the command shown is one that actually parses.

/** Subcommand for each step, and how its options are passed. */
const COMMANDS = {
  download_osm: { name: "download", style: "none" },
  elevation_tiles: { name: "elevation", style: "none" },
  basemap: { name: "basemap", style: "o" },
  routes: { name: "routes", style: "o" },
  terrain_rgb: { name: "terrain", style: "flags" },
  hillshade: { name: "hillshade", style: "flags" },
  valhalla_tiles: { name: "valhalla-tiles", style: "none" },
  valhalla_package: { name: "package", style: "flags" },
};

/** Quote an argument only when a shell would need it. */
export function quote(value) {
  const text = String(value);
  return /^[A-Za-z0-9_.,:/=+-]+$/.test(text) ? text : `'${text.replace(/'/g, "'\\''")}'`;
}

/**
 * The `alpimaps` line equivalent to one step as it is configured now.
 *
 * Returns null for a step with no command. Options left unset emit nothing, exactly as they do
 * in a run - the command shown is as sparse as the form is.
 */
export function commandFor(step, area, values = {}, defs = []) {
  const spec = COMMANDS[step];
  if (!spec) return null;

  const args = ["alpimaps", spec.name, "--area", quote(area || "AREA")];
  const byKey = new Map(defs.map((d) => [d.key, d]));
  for (const [key, value] of Object.entries(values)) {
    if (value === undefined || value === "") continue;
    const def = byKey.get(key);
    if (!def) continue;
    if (spec.style === "o") {
      args.push("-o", quote(`${key}=${value}`));
    } else if (spec.style === "flags") {
      // a boolean flag is its presence; false means "do not pass it"
      if (def.kind?.type === "bool") {
        if (value === true) args.push(`--${def.flag}`);
      } else {
        args.push(`--${def.flag}`, quote(value));
      }
    }
  }
  return args.join(" ");
}

/** Every configured step as one script, in the order they would run. */
export function scriptFor(steps, area, values = {}, defs = {}) {
  return steps
    .map((step) => commandFor(step, area, values[step] ?? {}, defs[step] ?? []))
    .filter(Boolean)
    .join("\n");
}
