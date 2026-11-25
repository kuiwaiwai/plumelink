import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { promisify } from 'node:util';
import { createServer } from 'node:net';
import { mkdtemp, mkdir, writeFile, copyFile } from 'node:fs/promises';
import { resolve, dirname, join } from 'node:path';

const run = promisify(execFile);
const sourceExe = resolve(process.argv[2] || 'src-tauri/target/release/PlumeLink.exe');
await mkdir('artifacts', { recursive: true });
const output = await mkdtemp(resolve('artifacts/usb-startup-'));
const app = join(output, '羽连 USB 测试');
const userHome = join(output, '用户配置');
await mkdir(join(app, 'usb'), { recursive: true });
await mkdir(userHome);
const exe = join(app, 'PlumeLink.exe');
await copyFile(sourceExe, exe);
for (const file of ['adb.exe', 'AdbWinApi.dll', 'AdbWinUsbApi.dll', 'NOTICE.txt'])
  await copyFile(join(dirname(sourceExe), 'usb', file), join(app, 'usb', file));
const env = { ...process.env };
for (const name of Object.keys(env)) {
  if (/^(path|android_home|android_sdk_root|localappdata|adb_server_socket|android_serial|adb_trace|android_adb_server_address|android_adb_server_port)$/i.test(name)) delete env[name];
}
env.Path = join(process.env.SystemRoot || 'C:/Windows', 'System32');
env.ANDROID_USER_HOME = userHome;
env.ADB_MDNS_AUTO_CONNECT = '0';
const inherited = { ...env, ADB_SERVER_SOCKET: 'tcp:127.0.0.1:1', ANDROID_ADB_SERVER_ADDRESS: '203.0.113.1', ANDROID_ADB_SERVER_PORT: '1', ANDROID_SERIAL: 'unrelated-device' };
const options = { env: inherited, cwd: output, windowsHide: true, timeout: 35000, encoding: 'utf8' };
function diagnose(port) {
  return new Promise((resolve,reject) => {
    const child=spawn(exe,['--verify-usb-connection',String(port)],{...options,stdio:['ignore','pipe','pipe']});
    let stdout='',stderr='';
    child.stdout.on('data',data=>stdout+=data);
    child.stderr.on('data',data=>stderr+=data);
    const timer=setTimeout(()=>{child.kill();reject(Error('USB diagnostic timed out'));},options.timeout);
    child.on('error',error=>{clearTimeout(timer);reject(error);});
    child.on('exit',code=>{
      clearTimeout(timer);
      const finish=()=>{
        child.stdout.destroy();child.stderr.destroy();
        if(code===0)resolve({stdout});else reject(Object.assign(Error('USB diagnostic failed'),{code,stdout,stderr}));
      };
      if(stdout.includes('\n'))finish();else setTimeout(finish,100);
    });
  });
}
const version = JSON.parse((await run(exe, ['--verify-usb-component'], options)).stdout);
assert.equal(version.ok, true);
let usbMayBeInUse = true;
try {
  const query = "@(Get-CimInstance Win32_PnPEntity -ErrorAction Stop | Where-Object { $_.PNPDeviceID -like 'USB\\*' -and ($_.Service -match 'WinUSB|Android|ADB' -or $_.Name -match 'ADB|Android') }).Count";
  const inspection = await run(join(process.env.SystemRoot, 'System32/WindowsPowerShell/v1.0/powershell.exe'), ['-NoProfile', '-NonInteractive', '-Command', query], { ...options, env, timeout: 20000 });
  const count = Number(inspection.stdout.trim());
  usbMayBeInUse = !Number.isInteger(count) || count > 0;
} catch {}
const lease = createServer();
await new Promise(resolve => lease.listen(0, '127.0.0.1', resolve));
const port = lease.address().port;
assert.notEqual(port, 5037);
await new Promise(resolve => lease.close(resolve));
const results = { unicodeVersion: version };
try {
  for (const name of usbMayBeInUse ? [] : ['cold', 'warm']) {
    const { stdout } = await diagnose(port);
    const result = JSON.parse(stdout);
    assert.equal(result.ok, true, stdout);
    assert.equal(result.stage, 'ready');
    assert.equal(typeof result.deviceCount, 'number');
    assert(!/serial|fingerprint|installed as|[A-Z]:\\/i.test(stdout));
    results[name] = result;
  }
} finally {
  if (!usbMayBeInUse) await run(join(dirname(exe), 'usb/adb.exe'), ['-P', String(port), 'kill-server'], { ...options, env, timeout: 10000 }).catch(() => {});
}
if (usbMayBeInUse) results.startupSkipped = 'A USB interface may be in use; avoid competing with an existing phone connection.';

const foreign = createServer(socket => {
  socket.on('data', () => socket.end('HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n'));
  socket.on('error', () => {});
});
await new Promise(resolve => foreign.listen(0, '127.0.0.1', resolve));
try {
  let rejected = false;
  try { await diagnose(foreign.address().port); }
  catch (error) {
    assert.equal(error.code, 1);
    results.foreignPort = JSON.parse(error.stdout);
    assert.equal(results.foreignPort.stage, 'server');
    assert.equal(results.foreignPort.ok, false);
    assert(results.foreignPort.error.includes('USB 后台服务未正常响应'));
    rejected = true;
  }
  assert(rejected, 'A non-ADB listener must not be accepted or replaced');
} finally { await new Promise(resolve => foreign.close(resolve)); }
await writeFile(join(output, 'results.json'), JSON.stringify({ ...results, unicodeInstallAndUserDirectory: true, sharedAdbPortUntouched: true }, null, 2));
console.log(`USB Unicode paths and conflicting listener passed; cold/warm start ${usbMayBeInUse ? 'skipped to preserve USB devices' : 'passed'}: ${output}`);
