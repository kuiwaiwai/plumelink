import { execFileSync } from 'node:child_process';
import { lstatSync, readFileSync, realpathSync } from 'node:fs';
import { dirname, relative, resolve, sep } from 'node:path';

const git = (args, input) => execFileSync('git', args, { input, maxBuffer: 64 * 1024 * 1024 });
const paths = args => git(args).toString('utf8').split('\0').filter(Boolean);
process.chdir(git(['rev-parse', '--show-toplevel']).toString('utf8').trimEnd());
const root = realpathSync('.');
const findings = new Set();
const maxBytes = 5_000_000;
const binaries = new Set([
  'android-app/gradle/wrapper/gradle-wrapper.jar',
  'android-app/app/src/main/res/drawable-nodpi/ic_launcher.png',
  'docs/assets/logo.png',
  ...[
    'desktop-overview', 'desktop-files', 'desktop-devices', 'desktop-preferences',
    'desktop-zh-hans', 'desktop-zh-hant', 'desktop-en',
    'android-touchpad-portrait', 'android-touchpad-landscape',
    'android-dpad-portrait', 'android-dpad-landscape',
  ].map(name => `docs/assets/ui/${name}.png`),
  ...['logo', 'logo-ui', 'favicon'].map(n => `shared/assets/${n}.png`),
  ...['icon.ico', 'icon.png', 'installer.ico', 'installer.png'].map(n => `src-tauri/icons/${n}`),
]);
const checks = [
  ['private key', /-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY(?: BLOCK)?-----/],
  ['access token', /\b(?:gh[pousr]_[\w]{20,}|github_pat_[\w]{30,}|AIza[\w-]{30,}|sk-[\w-]{20,})\b/],
  ['embedded credentials', /https?:\/\/[^/\s:@]+:[^/\s@]+@/i],
  ['literal npm credential', /^\s*(?:\/\/[^\s=]+:)?_auth(?:Token)?\s*=\s*["']?[^\s"'$`%]{4,}/im],
  ['literal signing password', /\b(?:PLUMELINK_(?:STORE|KEY)_PASSWORD|storePassword|keyPassword)\s*=\s*["'][^"'$%\r\n]+["']/i],
];
const sensitiveFile = /(?:^|\/)(?:\.env(?:\..*)?|(?:keystore|key|local|signing|secrets)\.properties|prefs\.json|adbkey(?:\.pub)?|id_(?:rsa|ed25519)(?:\.pub)?|[^/]*password[^/]*\.txt|[^/]+\.(?:jks|keystore|p12|pfx|pem|p8|asc|der|key(?:\.[^/]*)?|db|sqlite3?|log|dmp|pcapng?|hprof|jsonl|apk|aab|exe|zip|pdb))$/i;
function checkScreenshot(bytes) {
  let offset = 8;
  while (offset + 12 <= bytes.length) {
    const size = bytes.readUInt32BE(offset);
    const type = bytes.toString('ascii', offset + 4, offset + 8);
    if (['tEXt', 'zTXt', 'iTXt', 'eXIf'].includes(type)) return false;
    offset += 12 + size;
    if (type === 'IEND') return size === 0 && offset === bytes.length;
  }
  return false;
}
function inspect(file, bytes, files, source) {
  const flag = reason => findings.add(`${source}: ${file}: ${reason}`);
  if (sensitiveFile.test(file)) flag('private or generated filename');
  if (binaries.has(file)) {
    const magic = file.endsWith('.png') ? '89504e470d0a1a0a' : file.endsWith('.ico') ? '00000100' : '504b0304';
    if (!bytes.subarray(0, magic.length / 2).equals(Buffer.from(magic, 'hex'))) flag('unexpected asset format');
    if (file.startsWith('docs/assets/') && !checkScreenshot(bytes)) flag('PNG contains metadata, trailing data or invalid chunks');
    return;
  }
  let body;
  try { body = new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
  catch { return flag('unreviewed binary or non-UTF-8 file'); }
  if (body.includes('\0')) return flag('unreviewed binary');
  for (const match of body.matchAll(/["'](private_key|privateKey|token_digest|tokenDigest|token)["']\s*:\s*["'](dpapi:[^"']+|[\w+/=-]{32,})["']/gi)) {
    if (file === 'shared/protocol-cases.json' && match[1] === 'token' && match[2] === 'a'.repeat(64)) continue;
    flag('serialized credential');
  }
  for (const [name, pattern] of checks) if (pattern.test(body)) flag(name);
  const normalized = body.replaceAll('\\', '/').replace(/\/{2,}/g, '/');
  const homePaths = normalized.matchAll(/(?:\b[A-Z]:\/Users|(?<![\w:])\/Users|(?<!\w)\/home)\/([^/\s"`'<>]+)/gi);
  for (const [, user] of homePaths) {
    if (!/^(?:Demo|Test|Public|Default)$/i.test(user) && !user.startsWith('$')) flag('personal home path');
  }
  if (root.length > 3 && normalized.toLowerCase().includes(root.replaceAll('\\', '/').toLowerCase() + '/')) flag('absolute checkout path');
  if (process.env.COMPUTERNAME && body.toLowerCase().includes(process.env.COMPUTERNAME.toLowerCase())) flag('local machine name');
  if (!file.endsWith('.md')) return;
  for (const match of body.matchAll(/\]\(([^)]+)\)|<(?:img|a)\b[^>]*\b(?:src|href)="([^"]+)"/g)) {
    const target = (match[1] ?? match[2]).split('#', 1)[0];
    if (!target || /^(?:https?:|mailto:|\/\/)/i.test(target)) continue;
    const resolved = relative(root, resolve(dirname(file), target)).split(sep).join('/');
    if (!files.has(resolved)) flag('unpublished or missing link ' + target);
  }
}

for (const file of paths(['ls-files', '--cached', '--ignored', '--exclude-standard', '-z'])) {
  findings.add('index: ' + file + ': tracked despite ignore rules');
}
// Read objects from the index itself. A working-tree edit after git add cannot hide a staged secret.
const index = new Map();
for (const entry of paths(['ls-files', '--stage', '-z'])) {
  const i = entry.indexOf('\t');
  const [mode, oid, stage] = entry.slice(0, i).split(' ');
  const file = entry.slice(i + 1);
  if (stage !== '0' || !['100644', '100755'].includes(mode)) findings.add('index: ' + file + ': unresolved or non-regular entry');
  else index.set(file, oid);
}
const byOid = new Map();
for (const [file, oid] of index) {
  if (!byOid.has(oid)) byOid.set(oid, []);
  byOid.get(oid).push(file);
}
if (byOid.size) {
  let batch = [];
  let batchSize = 0;
  function scanBatch() {
    const result = git(['cat-file', '--batch'], batch.join('\n') + '\n');
    let start = 0;
    for (const oid of batch) {
      const end = result.indexOf(10, start);
      const [id, type, length] = result.toString('ascii', start, end).split(' ');
      if (id !== oid || type !== 'blob') throw new Error('Unexpected git cat-file response');
      start = end + 1;
      const bytes = result.subarray(start, start + Number(length));
      for (const file of byOid.get(oid)) inspect(file, bytes, index, 'index');
      start += bytes.length + 1;
    }
    batch = [];
    batchSize = 0;
  }
  const sizes = git(['cat-file', '--batch-check'], [...byOid.keys()].join('\n') + '\n').toString().trimEnd();
  for (const line of sizes.split('\n')) {
    const [oid, type, size] = line.split(' ');
    const names = byOid.get(oid);
    if (type !== 'blob' || Number(size) > maxBytes) {
      for (const file of names) findings.add('index: ' + file + ': oversized or unreadable object');
      continue;
    }
    if (batchSize + Number(size) > 16 * 1024 * 1024) scanBatch();
    batch.push(oid);
    batchSize += Number(size) + 100;
  }
  if (batch.length) scanBatch();
}
let workingCount = 0;
if (!process.argv.includes('--staged')) {
  const working = new Set(paths(['ls-files', '--cached', '--others', '--exclude-standard', '-z']).filter(f => lstatSync(f, { throwIfNoEntry: false })));
  for (const file of working) {
    const stat = lstatSync(file);
    if (!stat.isFile() || stat.size > maxBytes || relative(root, realpathSync(file)).split(sep).join('/') !== file) {
      findings.add('worktree: ' + file + ': oversized or non-regular file');
    } else inspect(file, readFileSync(file), working, 'worktree');
  }
  workingCount = working.size;
}
if (findings.size) {
  console.error('Public source audit failed:\n' + [...findings].join('\n'));
  process.exitCode = 1;
} else console.log(`Public source audit passed (${index.size} indexed, ${workingCount} working-tree files checked).`);
