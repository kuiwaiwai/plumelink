import { releaseRustEnv } from "./release-rust-env.mjs";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { prepareUsbComponent } from "./prepare-usb.mjs";
const local = join(process.env.USERPROFILE || "", ".cargo", "bin", "cargo.exe");
const args = process.argv.slice(2);
if (["build", "check", "test", "clippy", "run"].includes(args[0])) await prepareUsbComponent();
const separator = args.indexOf("--");
args.splice(separator < 0 ? args.length : separator, 0, "--manifest-path", "src-tauri/Cargo.toml");
const child = spawn(
  existsSync(local) ? local : "cargo",
  args,
  { env: args.includes("--release") ? releaseRustEnv() : process.env, stdio: "inherit" },
);
child.on("error", (e) => {
  console.error(e);
  process.exitCode = 1;
});
child.on("exit", (code) => (process.exitCode = code ?? 1));
