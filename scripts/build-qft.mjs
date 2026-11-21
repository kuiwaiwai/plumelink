import { releaseRustEnv } from "./release-rust-env.mjs";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, copyFileSync } from "node:fs";
import { join, resolve } from "node:path";
const cargo = join(process.env.USERPROFILE || process.env.HOME || "", ".cargo", "bin", process.platform === "win32" ? "cargo.exe" : "cargo");
const lint = process.argv.includes("--lint");
const args = [lint ? "clippy" : "build", "--release", "--locked", "--target", "wasm32-unknown-unknown", "--manifest-path", "qft-web/Cargo.toml"];
if (lint) args.push("--", "-D", "warnings");
const result = spawnSync(existsSync(cargo) ? cargo : "cargo", args, { env: releaseRustEnv(), stdio: "inherit", windowsHide: true });
if (result.status !== 0) throw new Error("QFT WebAssembly 构建失败；请安装 rustup target add wasm32-unknown-unknown");
if (!lint) {
  mkdirSync(resolve("apps/mobile/generated"), { recursive: true });
  copyFileSync("qft-web/target/wasm32-unknown-unknown/release/plumelink_qft_web.wasm", "apps/mobile/generated/qft.wasm");
}
