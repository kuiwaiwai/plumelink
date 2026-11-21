import { homedir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Cargo applies these flags to dependencies too, including their panic locations.
export function releaseRustEnv(env = process.env) {
  const flags = env.CARGO_ENCODED_RUSTFLAGS?.split("\x1f") ?? (env.RUSTFLAGS || "").split(/\s+/).filter(Boolean);
  const home = homedir();
  const roots = [
    [home, "/build/home"],
    [env.CARGO_HOME || resolve(home, ".cargo"), "/build/cargo"],
    [env.RUSTUP_HOME || resolve(home, ".rustup"), "/build/rustup"],
    [fileURLToPath(new URL("../", import.meta.url)), "/build/plumelink"],
  ];
  for (const [root, prefix] of roots) {
    for (const path of new Set([resolve(root), resolve(root).replaceAll("\\", "/")])) {
      const flag = `--remap-path-prefix=${path}=${prefix}`;
      if (!flags.includes(flag)) flags.push(flag);
    }
  }
  return { ...env, CARGO_ENCODED_RUSTFLAGS: flags.join("\x1f") };
}
