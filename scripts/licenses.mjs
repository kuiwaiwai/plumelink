import { readFile, readdir, writeFile, mkdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { execFileSync } from "node:child_process";

const sharedTexts = new Map();
const sharedList = [];

function refLicenseText(raw) {
  const normalized = raw.replace(/\r\n/g, "\n").trim();
  if (!normalized) return "";
  let id = sharedTexts.get(normalized);
  if (!id) {
    id = `L${sharedList.length + 1}`;
    sharedTexts.set(normalized, id);
    sharedList.push({ id, text: normalized });
  }
  return `[See License #${id}]`;
}

async function collectNotices(dir) {
  const refs = [];
  for (const file of await readdir(dir, { withFileTypes: true })) {
    if (!/^(licen[cs]e|copying|notice|copyright)/i.test(file.name)) continue;
    const full = join(dir, file.name);
    if (file.isDirectory()) {
      refs.push(...(await collectNotices(full)));
    } else {
      const ref = refLicenseText(await readFile(full, "utf8"));
      if (ref) refs.push(`${file.name}: ${ref}`);
    }
  }
  return refs;
}

const SKIP_JS = new Set([
  "@vue/compiler-sfc",
  "@vue/server-renderer",
  "yargs",
]);

const entries = [];

const seenJs = new Set();
async function collectJs(name, parent = process.cwd()) {
  if (SKIP_JS.has(name)) return;
  let current = parent;
  let dir;
  while (true) {
    const candidate = join(current, "node_modules", name);
    if (existsSync(join(candidate, "package.json"))) {
      dir = candidate;
      break;
    }
    const up = dirname(current);
    if (up === current) throw Error(`Missing dependency ${name}`);
    current = up;
  }
  if (seenJs.has(dir)) return;
  seenJs.add(dir);
  const pkg = JSON.parse(await readFile(join(dir, "package.json"), "utf8"));
  const refs = await collectNotices(dir);
  entries.push(`${pkg.name} ${pkg.version} (${pkg.license || "see notice"})${refs.length ? "\n" + refs.join("\n") : ""}`);
  for (const dep of Object.keys(pkg.dependencies || {})) {
    await collectJs(dep, dir);
  }
}

for (const name of ["vue", "qrcode", "@tauri-apps/api", "@lucide/vue", "zod"]) {
  await collectJs(name);
}

const localCargo = join(process.env.USERPROFILE || "", ".cargo", "bin", "cargo.exe");
const metadata = JSON.parse(
  execFileSync(
    existsSync(localCargo) ? localCargo : "cargo",
    [
      "metadata",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--format-version",
      "1",
      "--locked",
      "--filter-platform",
      "x86_64-pc-windows-msvc",
    ],
    { encoding: "utf8", maxBuffer: 20 * 1024 * 1024 },
  ),
);

for (const pkg of metadata.packages
  .filter(p => p.source || p.name === "qfast-transmission")
  .sort((a, b) => a.name.localeCompare(b.name))) {
  const manifestDir = dirname(pkg.manifest_path);
  const refs = await collectNotices(manifestDir);
  if (pkg.license_file) {
    const ref = refLicenseText(await readFile(join(manifestDir, pkg.license_file), "utf8"));
    if (ref) refs.push(`${pkg.license_file}: ${ref}`);
  }
  const uniqueRefs = [...new Set(refs)];
  entries.push(`${pkg.name} ${pkg.version} (${pkg.license || "see source"}) ${pkg.repository || ""}${uniqueRefs.length ? "\n" + uniqueRefs.join("\n") : ""}`);
}

const sections = [
  "羽连 PlumeLink — Third-party licenses\n",
  entries.join("\n\n"),
  "\n\n==================== SHARED LICENSE TEXTS ====================\n",
  sharedList.map(item => `----- #${item.id} -----\n${item.text}`).join("\n\n"),
];

await mkdir("tauri-dist", { recursive: true });
await writeFile("tauri-dist/THIRD_PARTY_LICENSES.txt", sections.join("\n") + "\n");
console.log(`Generated license file (${sharedList.length} unique license texts, ${entries.length} packages).`);
