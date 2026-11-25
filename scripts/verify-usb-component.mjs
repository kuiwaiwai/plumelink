import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, mkdtemp, rmdir } from 'node:fs/promises';
import { dirname, resolve, join } from 'node:path';
import { tmpdir } from 'node:os';
import { execFileSync } from 'node:child_process';

const lock = JSON.parse(await readFile(new URL('./usb-component.json', import.meta.url), 'utf8'));
const exe = resolve(process.argv[2] || 'src-tauri/target/release/PlumeLink.exe');
for (const [name, expected] of Object.entries(lock.files)) {
  const data = await readFile(join(dirname(exe), 'usb', name));
  assert.equal(createHash('sha256').update(data).digest('hex'), expected, name);
}
const cwd = await mkdtemp(join(tmpdir(), 'PlumeLink USB check '));
try {
  const env = { ...process.env };
  for (const name of Object.keys(env)) {
    if (/^(path|android_home|android_sdk_root|adb_server_socket|android_serial|adb_trace|localappdata)$/i.test(name)) delete env[name];
  }
  env.Path = join(process.env.SystemRoot || 'C:/Windows', 'System32');
  const result = JSON.parse(execFileSync(exe, ['--verify-usb-component'], { env, cwd, encoding: 'utf8', timeout: 15000, windowsHide: true }));
  assert.equal(result.ok, true);
  assert.equal(result.component, 'bundled');
  assert(result.version.includes(`Version ${lock.version}-`), result.version);
  console.log(`Bundled USB ${lock.version}: file hashes and execution without SDK/PATH passed.`);
} finally { await rmdir(cwd); }
