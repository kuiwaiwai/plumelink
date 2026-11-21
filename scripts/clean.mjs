import { rm } from "node:fs/promises";
import { resolve } from "node:path";

const targets = [
  "artifacts",
  "output",
  "tauri-dist",
  "QFastTransmission/target",
  "android-app/build",
  "android-app/app/build",
];

for (const rel of targets) {
  await rm(resolve(rel), { recursive: true, force: true });
  console.log(`Cleaned ${rel}`);
}
