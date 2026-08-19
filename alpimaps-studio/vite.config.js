import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5183, strictPort: true },
  // maplibre-gl-compare is a CommonJS module that does require("events").EventEmitter, which
  // has no browser equivalent - without the polyfill the constructor throws
  // "EventEmitter is not a constructor" and no swiper is ever created.
  resolve: { alias: { events: "events" } },
  // No `optimizeDeps.exclude` for maplibre-gl. That was needed against 6.x, whose multi-file
  // .mjs dist confused the optimiser into serving a worker URL that 404'd - and a missing
  // worker means no tile is ever parsed, with no error to show for it. 5.x ships a single CJS
  // bundle that pre-bundles cleanly, worker included, and excluding it here would instead break
  // named-export extraction from that CJS.
});
