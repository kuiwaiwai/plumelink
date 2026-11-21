import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";
import { build } from "vite";
import vue from "@vitejs/plugin-vue";

const skipQft = process.argv.includes("--skip-qft") && existsSync(resolve("apps/mobile/generated/qft.wasm"));
if (!skipQft) {
  const result = spawnSync(process.execPath, [resolve("scripts/build-qft.mjs")], {
    stdio: "inherit",
    windowsHide: true,
  });
  if (result.status !== 0) process.exit(result.status || 1);
}

for (const app of ["mobile", "desktop-ui"]) {
  const target = app === "mobile" ? "chrome85" : "chrome111";
  await build({
    configFile: false,
    root: resolve("apps", app),
    base: "./",
    plugins: [vue()],
    build: { target, cssTarget: target, outDir: resolve("tauri-dist", app), emptyOutDir: true },
  });
}

copyFileSync(resolve("PRIVACY.md"), resolve("tauri-dist/mobile/PRIVACY.md"));
