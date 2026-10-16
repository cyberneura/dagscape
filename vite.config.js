import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
// @ts-expect-error node の型定義を入れていない
import { writeFileSync } from "node:fs";

// @ts-expect-error process is a nodejs global
const bundledModulesOut = process.env.DAGSCAPE_BUNDLED_MODULES_OUT;

// scripts/generate-third-party-notices.sh が DAGSCAPE_BUNDLED_MODULES_OUT を付けて build し、
// web view の bundle に実際に入ったモジュールを読む (package.json の依存とは一致しないため)。
// 配布されるのは client の出力だけで、SSR の出力は prerender にしか使わない
/** @returns {import("vite").Plugin} */
function listBundledModules() {
  /** @type {Set<string>} */
  const ids = new Set();
  return {
    name: "dagscape-list-bundled-modules",
    apply: "build",
    generateBundle(_options, bundle) {
      if (this.environment.name !== "client") return;
      for (const chunk of Object.values(bundle)) {
        if (chunk.type === "chunk") for (const id of chunk.moduleIds) ids.add(id);
      }
    },
    closeBundle() {
      if (ids.size > 0) writeFileSync(bundledModulesOut, JSON.stringify([...ids].sort()));
    },
  };
}

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [tailwindcss(), sveltekit(), ...(bundledModulesOut ? [listBundledModules()] : [])],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
