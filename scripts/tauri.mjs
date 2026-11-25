import { releaseRustEnv } from "./release-rust-env.mjs";
import { verifyDesktopPermissions } from "./desktop-permissions.mjs";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { prepareUsbComponent } from "./prepare-usb.mjs";
if (["build", "bundle", "dev"].includes(process.argv[2])) await prepareUsbComponent();
if (["build", "bundle", "dev"].includes(process.argv[2])) verifyDesktopPermissions();
const env = process.argv.includes("build") ? releaseRustEnv() : { ...process.env };
const cargoBin = join(env.USERPROFILE || env.HOME || "", ".cargo", "bin");
const pathKey =
  Object.keys(env).find((k) => k.toLowerCase() === "path") || "Path";
if (existsSync(join(cargoBin, "cargo.exe")))
  env[pathKey] = cargoBin + ";" + env[pathKey];
const child = spawn(
  process.execPath,
  ["node_modules/@tauri-apps/cli/tauri.js", ...process.argv.slice(2)],
  { env, stdio: "inherit" },
);
child.on("error", (error) => {
  console.error(error);
  process.exitCode = 1;
});
child.on("exit", (code) => (process.exitCode = code ?? 1));
