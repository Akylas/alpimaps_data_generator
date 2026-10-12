<script>
  // Elevation profile chart. Plain SVG - the data is a few hundred points and a charting
  // library would be more bytes than the whole app.
  let { profile } = $props();

  const W = 900, H = 190, PAD = { l: 46, r: 10, t: 10, b: 22 };

  let pts = $derived((profile?.points ?? []).filter((p) => p.elevation_m != null));
  let lo = $derived(profile?.min_m ?? 0);
  let hi = $derived(profile?.max_m ?? 1);
  let span = $derived(Math.max(hi - lo, 1));

  function x(d) {
    const total = profile?.distance_m || 1;
    return PAD.l + (d / total) * (W - PAD.l - PAD.r);
  }
  function y(e) {
    return PAD.t + (1 - (e - lo) / span) * (H - PAD.t - PAD.b);
  }

  let path = $derived(
    pts.length ? pts.map((p, i) => `${i ? "L" : "M"}${x(p.distance_m).toFixed(1)},${y(p.elevation_m).toFixed(1)}`).join(" ") : ""
  );
  let area = $derived(
    pts.length ? `${path} L${x(pts.at(-1).distance_m).toFixed(1)},${H - PAD.b} L${x(pts[0].distance_m).toFixed(1)},${H - PAD.b} Z` : ""
  );
  let ticks = $derived([lo, lo + span / 2, hi]);
</script>

{#if profile && pts.length}
  <div class="stats">
    <span><b>{(profile.distance_m / 1000).toFixed(2)}</b> km</span>
    <span class="up">+{profile.ascent_m.toFixed(0)} m</span>
    <span class="down">−{profile.descent_m.toFixed(0)} m</span>
    <span>{profile.min_m.toFixed(0)}–{profile.max_m.toFixed(0)} m</span>
    <span class="muted">z{profile.zoom} · {pts.length} samples · ±{profile.threshold_m} m threshold</span>
    {#if profile.gaps}<span class="warn">{profile.gaps} gaps</span>{/if}
  </div>

  <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none">
    {#each ticks as t}
      <line x1={PAD.l} x2={W - PAD.r} y1={y(t)} y2={y(t)} class="grid" />
      <text x={PAD.l - 6} y={y(t) + 3} class="axis">{t.toFixed(0)}</text>
    {/each}
    <path d={area} class="fill" />
    <path d={path} class="line" />
    <text x={PAD.l} y={H - 6} class="axis start">0</text>
    <text x={W - PAD.r} y={H - 6} class="axis end">{(profile.distance_m / 1000).toFixed(1)} km</text>
  </svg>
{/if}

<style>
  .stats { display: flex; gap: 14px; align-items: baseline; font-size: 13px;
           font-variant-numeric: tabular-nums; margin-bottom: 6px; flex-wrap: wrap; }
  .stats b { font-size: 15px; }
  .up { color: var(--ok); }
  .down { color: var(--warn); }
  .muted { color: var(--muted-2); font-size: 12px; }
  .warn { color: var(--warn); }
  svg { width: 100%; height: 190px; display: block; }
  .grid { stroke: var(--line-2); stroke-width: 1; }
  .axis { fill: var(--muted-2); font-size: 10px; text-anchor: end; }
  .axis.start { text-anchor: start; }
  .axis.end { text-anchor: end; }
  .fill { fill: var(--accent); opacity: 0.35; }
  .line { fill: none; stroke: var(--ok); stroke-width: 1.6; }
</style>
