import { createHash, randomUUID } from 'node:crypto';
import { readFile, writeFile, mkdir, rename, rm, lstat, realpath } from 'node:fs/promises';
import { resolve, dirname, basename } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { inflateRawSync } from 'node:zlib';

const root = fileURLToPath(new URL('../', import.meta.url));
const digest = data => createHash('sha256').update(data).digest('hex');
const lock = JSON.parse(await readFile(new URL('./usb-component.json', import.meta.url), 'utf8'));

function extract(data) {
  let end = data.length - 22;
  while (end >= Math.max(0, data.length - 65557) && data.readUInt32LE(end) !== 0x06054b50) end--;
  if (end < 0 || data.readUInt32LE(end) !== 0x06054b50) throw Error('Invalid USB archive');
  const result = new Map(), count = data.readUInt16LE(end + 10);
  let offset = data.readUInt32LE(end + 16);
  for (let i = 0; i < count; i++) {
    if (data.readUInt32LE(offset) !== 0x02014b50) throw Error('Invalid USB archive entry');
    const method = data.readUInt16LE(offset + 10), size = data.readUInt32LE(offset + 20);
    const nameSize = data.readUInt16LE(offset + 28), extraSize = data.readUInt16LE(offset + 30), commentSize = data.readUInt16LE(offset + 32);
    const path = data.toString('utf8', offset + 46, offset + 46 + nameSize);
    const name = path.startsWith('platform-tools/') ? path.slice(15) : '';
    if (Object.hasOwn(lock.files, name)) {
      if (result.has(name) || data.readUInt16LE(offset + 8) & 1) throw Error('Invalid USB archive file');
      const local = data.readUInt32LE(offset + 42);
      if (data.readUInt32LE(local) !== 0x04034b50) throw Error('Invalid USB local entry');
      const start = local + 30 + data.readUInt16LE(local + 26) + data.readUInt16LE(local + 28);
      const compressed = data.subarray(start, start + size);
      const bytes = method === 0 ? compressed : method === 8 ? inflateRawSync(compressed, { maxOutputLength: 16 * 1024 * 1024 }) : null;
      if (!bytes || digest(bytes) !== lock.files[name]) throw Error(`USB file checksum failed: ${name}`);
      result.set(name, bytes);
    }
    offset += 46 + nameSize + extraSize + commentSize;
  }
  if (result.size !== Object.keys(lock.files).length) throw Error('Incomplete USB archive');
  return result;
}

async function verified(directory) {
  try {
    if (!(await lstat(directory)).isDirectory() || (await lstat(directory)).isSymbolicLink()) return false;
    for (const [name, sha256] of Object.entries(lock.files))
      if (!(await lstat(resolve(directory, name))).isFile() || digest(await readFile(resolve(directory, name))) !== sha256) return false;
    return true;
  } catch { return false; }
}

async function removeStage(directory) {
  if (dirname(resolve(directory)) !== resolve(root, 'tauri-dist') || !/^\.usb-(old-)?[0-9a-f-]+$/.test(basename(directory)))
    throw Error('Unexpected USB staging path');
  if ((await realpath(dirname(directory))).toLowerCase() !== resolve(root, 'tauri-dist').toLowerCase())
    throw Error('USB staging directory is outside the output directory');
  const item = await lstat(directory).catch(error => { if (error.code !== 'ENOENT') throw error; return null; });
  if (!item) return;
  if (!item.isDirectory() || item.isSymbolicLink()) throw Error('Invalid USB staging directory');
  await rm(directory, { recursive: true });
}

export async function prepareUsbComponent() {
  const destination = resolve(root, 'tauri-dist/usb');
  if (await verified(destination)) return;
  const cache = resolve(root, 'artifacts/usb-component'), archive = resolve(cache, `platform-tools-${lock.version}-win.zip`);
  await mkdir(cache, { recursive: true });
  let data = await readFile(archive).catch(() => null);
  if (!data || data.length !== lock.bytes || digest(data) !== lock.sha256) {
    const response = await fetch(lock.url, { signal: AbortSignal.timeout(90000) });
    if (!response.ok) throw Error(`USB component download failed: ${response.status}`);
    data = Buffer.from(await response.arrayBuffer());
    if (data.length !== lock.bytes || digest(data) !== lock.sha256) throw Error('USB archive checksum failed');
    const temporary = archive + '.' + randomUUID();
    await writeFile(temporary, data, { flag: 'wx' });
    await rename(temporary, archive);
  }
  const files = extract(data), stage = resolve(root, 'tauri-dist/.usb-' + randomUUID());
  await mkdir(dirname(stage), { recursive: true });
  if ((await realpath(dirname(stage))).toLowerCase() !== dirname(stage).toLowerCase()) throw Error('Invalid USB output directory');
  await mkdir(stage);
  try {
    for (const [name, bytes] of files) await writeFile(resolve(stage, name), bytes, { flag: 'wx' });
    if (!await verified(stage)) throw Error('USB staging verification failed');
    const existing = await lstat(destination).catch(() => null);
    if (existing) {
      if (!existing.isDirectory() || existing.isSymbolicLink()) throw Error('Invalid USB output directory');
      const previous = resolve(root, 'tauri-dist/.usb-old-' + randomUUID());
      await rename(destination, previous);
      try { await rename(stage, destination); }
      catch (error) { await rename(previous, destination); throw error; }
      await removeStage(previous);
    } else await rename(stage, destination);
  } finally { await removeStage(stage); }
  console.log(`Prepared bundled USB component ${lock.version}.`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await prepareUsbComponent();
