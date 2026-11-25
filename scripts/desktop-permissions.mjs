import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

export function verifyDesktopPermissions(root = new URL('../', import.meta.url)) {
  const read = path => readFileSync(new URL(path, root), 'utf8');
  const handler = read('src-tauri/src/main.rs').match(/tauri::generate_handler!\s*\[([\s\S]*?)\]/);
  const manifest = read('src-tauri/build.rs').match(/AppManifest::new\(\)\s*\.commands\(\s*&\s*\[([\s\S]*?)\]/);
  assert(handler && manifest, 'Cannot inspect the desktop command registry');
  const commands = [...handler[1].matchAll(/^\s*(\w+)\s*,?\s*$/gm)].map(match => match[1]);
  const declared = [...manifest[1].matchAll(/"(\w+)"/g)].map(match => match[1]);
  assert(commands.length && new Set(commands).size === commands.length, 'Invalid desktop command registry');
  assert.deepEqual([...declared].sort(), [...commands].sort(), 'Desktop commands and permission manifest differ');
  const capability = JSON.parse(read('src-tauri/capabilities/desktop.json'));
  assert.equal(capability.local, true, 'Desktop commands must remain local');
  assert.deepEqual(capability.windows, ['main'], 'Desktop commands must be limited to the main window');
  assert(!capability.remote, 'Desktop commands must not be granted to remote pages');
  const allowed = capability.permissions.filter(item => typeof item === 'string' && !item.includes(':'));
  assert.deepEqual(allowed.sort(), commands.map(command => 'allow-' + command.replaceAll('_', '-')).sort(),
    'Desktop capability is missing a command permission or grants an unexpected one');
  const config = JSON.parse(read('src-tauri/tauri.conf.json'));
  assert(config.app.security.capabilities.includes(capability.identifier), 'Desktop capability is not enabled');
  return commands;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log(`Desktop command permissions checked (${verifyDesktopPermissions().length} commands).`);
}
