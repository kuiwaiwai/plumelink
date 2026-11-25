import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";

const npmCli = process.env.npm_execpath;
if (!npmCli) throw new Error("Run this script via npm run verify:regression.");
const steps = [
  [process.execPath, ["scripts/verify-public-source.mjs"], "public source privacy audit"],
  [process.execPath, [npmCli, "run", "typecheck"], "TypeScript typecheck"],
  [process.execPath, [npmCli, "test"], "frontend tests"],
  [process.execPath, [npmCli, "run", "build:frontend"], "production frontend build"],
  [process.execPath, [npmCli, "run", "check:rust"], "Rust check"],
  [process.execPath, [npmCli, "run", "lint:rust"], "Rust Clippy"],
  [process.execPath, [npmCli, "run", "lint:qft"], "WASM adapter Clippy"],
  [process.execPath, [npmCli, "run", "test:rust"], "Rust tests"],
  [process.execPath, ["scripts/verify-qft.mjs"], "QFastTransmission WASM/WSS integration"],
  [process.execPath, ["scripts/verify-qft.mjs", "--mixed"], "audio/input/file concurrency"],
  [process.execPath, ["scripts/verify-qft.mjs", "--native-ui"], "Android native UI bridge verification"],
  [process.execPath, ["scripts/verify-desktop-ui.mjs"], "desktop UI verification"],
];
if (!process.argv.includes("--core")) steps.push(
  [process.execPath, [npmCli, "run", "build"], "desktop build"],
  [process.execPath, [npmCli, "run", "verify:desktop"], "desktop lifecycle verification"],
  [process.execPath, ["scripts/verify-mobile-package.mjs"], "mobile package verification"],
);
if (process.argv.includes("--android")) steps.push(
  [process.execPath, [npmCli, "run", "build:android"], "Android debug build"],
);
const report = { startedAt: new Date().toISOString(), platform: process.platform, node: process.version, scope: process.argv.includes("--core") ? "core automation" : "Windows automation", passed: false, steps: [] };
mkdirSync("artifacts", { recursive: true });
const save = () => writeFileSync("artifacts/regression-results.json", JSON.stringify(report, null, 2));
for (const [command, args, label] of steps) {
  console.log(`\n[regression] ${label}`);
  const started = Date.now();
  const result = spawnSync(command, args, {
    stdio: "inherit",
    env: process.env,
    windowsHide: true,
    timeout: 30 * 60 * 1000,
  });
  report.steps.push({ label, durationMs: Date.now() - started, exitCode: result.status, error: result.error?.message || null });
  save();
  if (result.error) {
    console.error(`[regression] ${label} could not start: ${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    console.error(`[regression] ${label} failed with exit code ${result.status ?? "unknown"}`);
    process.exit(result.status || 1);
  }
}
report.passed = true;
report.finishedAt = new Date().toISOString();
save();
console.log("\n[regression] all automated checks passed");
