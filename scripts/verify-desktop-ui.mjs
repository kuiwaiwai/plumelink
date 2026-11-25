// Production desktop assets with a simulated Tauri bridge; no native actions.
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep, dirname } from 'node:path';
import { randomUUID } from 'node:crypto';
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';
import QRCode from 'qrcode';
import { verifyDesktopConnections } from './verify-desktop-connections.mjs';
import { verifyDesktopReceive } from './verify-desktop-receive.mjs';

const root = resolve('tauri-dist/desktop-ui');
const output = resolve(process.argv.find(arg => arg.startsWith('--output='))?.slice('--output='.length) || `artifacts/desktop-ui-${new Date().toISOString().replace(/[:.]/g, '-')}-${randomUUID().slice(0, 8)}`);
await mkdir(dirname(output), { recursive: true });
await mkdir(output);
const server = createServer(async (request, response) => {
  const path = resolve(root, '.' + (request.url === '/' ? '/index.html' : request.url.split('?')[0]));
  if (!path.startsWith(root + sep)) { response.writeHead(403).end(); return; }
  try {
    response.setHeader('Content-Type', ({ '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png', '.svg': 'image/svg+xml' })[extname(path)] || 'application/octet-stream');
    response.end(await readFile(path));
  } catch { response.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
let browser;
const errors = [];
const checks = [];
const config = JSON.parse(await readFile('src-tauri/tauri.conf.json', 'utf8'));
assert.equal(config.app.windows[0].decorations, false);
const capability = JSON.parse(await readFile('src-tauri/capabilities/desktop.json', 'utf8'));
for (const action of ['is-maximized', 'hide', 'toggle-maximize', 'start-dragging', 'close']) {
  assert(capability.permissions.includes(`core:window:allow-${action}`));
}
try {
  browser = await chromium.launch({ channel: process.env.PLAYWRIGHT_CHANNEL || 'chrome', headless: true });
  const context = await browser.newContext({ locale: "zh-CN", viewport: { width: 760, height: 760 } });
  await context.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { async writeText(text) { window.__clipboard = text; } } });
    window.__failState = true;
    window.__windowCalls = [];
    window.__maximized = false;
    window.__failRename = true;
    window.__audioSelection = '';
    window.__gainDb = 6;
    window.__gainRequests = [];
    window.__endpointVolumes = {};
    window.__verifiedInput = null;
    window.__haloRequests = [];
    window.__pickedFiles = [];
    window.__usbCalls = [];
    window.__usb = { available: true, busy: false, activeCount: 0, endpoint: '', fingerprint: '', code: '', error: '', devices: [] };
    window.__failTransfer = false;
    window.__recordingInputs = [{ id: 'uu-mic', name: '麦克风阵列 (UU远程虚拟音频设备)' }];
    window.__state = {
      running: true, discovery: { active: true, error: '' }, halo: { enabled: true, size: 'medium' },
      autostart: false, trusted: false, pairedCount: 0, devices: [], controlMode: 'free',
      primaryDevice: null, controlRequest: null, connected: false, connectedCount: 0,
      recentController: null, code: '012345', codeRemaining: 240, pairingIntervalMinutes: 15,
      addresses: [{ name: 'Wi-Fi', address: '192.168.1.20', virtual: false }], selected: '192.168.1.20',
      port: 17840, error: '', receivePath: 'Downloads\\羽连', sharedFiles: [],
      admin: { active: false },
      microphone: { active: false, deviceName: '', endpointName: '', error: '' },
    };
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
      async invoke(command, args) {
        if (command === 'receive_action') {
          (window.__receiveCalls ||= []).push(structuredClone(args));
          if (args.action === 'open') {
            const snapshot = structuredClone(window.__state);
            if (window.__holdReceiveOpen) await new Promise(resolve => { window.__finishReceiveOpen = resolve; });
            if (window.__failReceiveOpen) throw new Error('接收目录不存在或已被移动，请重新选择目录');
            return snapshot;
          }
          if (args.action === 'cancel') {
            if (window.__failReceiveCancel) throw new Error('无法清理未完成文件，请重试停止');
            const task = window.__state.receivedFiles.find(file => file.taskId === args.taskId);
            Object.assign(task, { state: 'stopping', error: undefined });
          }
          if (args.action === 'clear') window.__state.receivedFiles = window.__state.receivedFiles.filter(file => !['completed', 'stopped'].includes(file.state));
          return structuredClone(window.__state);
        }
        if (command === 'usb_action') {
          if (args.action === 'status' && window.__usbDetectionFails) throw new Error('USB 检测失败，请重试');
          if (args.action !== 'status') window.__usbCalls.push(structuredClone(args));
          if (args.action === 'connect') {
            window.__state.running = true; window.__state.lanRunning = false;
            Object.assign(window.__usb.devices.find(device => device.id === args.id),{linked:true,selected:true});
            Object.assign(window.__usb, { activeCount: 1, endpoint: '127.0.0.1:49123', fingerprint: Array(32).fill('AB').join(':'), code: '246802' });
          }
          if (args.action === 'disconnect') { window.__usb.devices.filter(device => !args.id || device.id === args.id).forEach(device => Object.assign(device,{linked:false,selected:false})); window.__usb.activeCount = window.__usb.devices.filter(device => device.selected).length; }
          return structuredClone(window.__usb);
        }
        if (command.startsWith('plugin:window|')) {
          const action = command.split('|')[1];
          if (action === 'is_maximized') return window.__maximized;
          window.__windowCalls.push(action);
          if (window.__stallWindow) return new Promise(() => {});
          if (window.__failWindow) throw new Error('simulated window failure');
          if (action === 'toggle_maximize') window.__maximized = !window.__maximized;
          return;
        }
        if (command === 'desktop_state') {
          if (window.__failState) { window.__failState = false; throw new Error('simulated unavailable bridge'); }
        } else if (command === 'set_service_port') {
          if (args.port === 19828) throw Error('端口 19828 已被占用，请选择其他端口。');
          window.__state.port = args.port;
        } else if (command === 'device_action') {
          const device = window.__state.devices.find(device => device.id === args.id);
          if (args.action === 'permissions') device.permissions = structuredClone(args.permissions);
          if (args.action === 'rename') {
            if (window.__failRename) throw new Error('保存失败测试');
            device.name = args.name;
          }
          if (args.action === 'permissions') device.permissions = args.permissions;
        } else if (command === 'microphone_action') {
          if (args.action === 'levels') {
            if (window.__failLevels) throw Error('读取设备音量超时，请稍后刷新。');
            const read = id => id ? { id, volumePercent: 100, muted: false, error: '', ...window.__endpointVolumes[id] } : null;
            return { output: read(args.id), recording: read(args.recording) };
          }
          if (args.action === 'gain') {
            window.__gainRequests.push(Number(args.id));
            if (window.__holdGain) await new Promise(resolve => { window.__finishGain = resolve; });
            if (window.__failGain) throw Error('增益保存失败测试');
            window.__gainDb = Number(args.id);
            return { gainDb: window.__gainDb };
          }
          if (args.action === 'setup') {
            if (!['cable', 'plume'].includes(args.id)) throw Error('No verified cable');
            window.__audioSelection = args.id;
            return {selected: args.id, target: {name: args.id === 'plume' ? 'Microphone (PlumeLink Microphone)' : 'CABLE Output (test)'}};
          }
          if (args.action === 'installCable') {
            window.__cableInstallRequests = (window.__cableInstallRequests || 0) + 1;
            if (window.__holdCableInstall) await new Promise(resolve => { window.__finishCableInstall = resolve; });
            if (window.__cancelInstaller) throw Error('已取消管理员授权。');
            window.__installedCable = true;
          }
          if (args.action === 'select') window.__audioSelection = args.id;
          if (args.action === 'stop') {
            if (window.__failAudioStop) throw Error('停止接收失败测试');
            window.__state.microphone.active = false;
            return { stopped: true };
          }
          const value = { cableInstalled: window.__serviceOnly || window.__installedCable, outputs: [{ id: 'plume', name: 'Speakers (PlumeLink Microphone)', virtualHint: true, plumeLink: true }, { id: 'cable', name: 'CABLE Input (test)', virtualHint: true }, { id: 'uu', name: '扬声器 (UU远程虚拟音频设备)', virtualHint: true }].filter(o => o.id !== 'cable' || window.__installedCable), recordingInputs: structuredClone(window.__recordingInputs), selected: window.__audioSelection, gainDb: window.__gainDb, verifiedInput: structuredClone(window.__verifiedInput) };
          if (args.action === 'list' && window.__holdAudioList) return new Promise(resolve => { window.__finishAudioList = () => resolve(value); });
          return value;
        } else if (command === 'application_action') {
          window.__applications ||= [];
          (window.__applicationCalls ||= []).push(structuredClone(args));
          if (args.action === 'add' && window.__failApplicationAdd) throw new Error('Add failed in test');
          if (args.action === 'remove' && window.__failApplicationRemove) throw new Error('Remove failed in test');
          if (args.action === 'add') window.__applications.push({ id: 'a'.repeat(32), name: 'Sample App', path: 'Applications/Sample App.exe' });
          if (args.action === 'add-folder' && !window.__cancelFolderPicker) window.__applications.push({ id: 'b'.repeat(32), name: '工作资料 (Q4) & Notes', path: 'Documents/工作资料 (Q4) & Notes', kind: 'folder' });
          if (args.action === 'remove') window.__applications = window.__applications.filter(entry => entry.id !== args.id);
          if (args.action === 'rename') {
            if (window.__failApplicationRename) throw new Error('Rename failed in test');
            window.__applications.find(entry => entry.id === args.id).name = args.name.trim();
          }
          return structuredClone(window.__applications);
        } else if (command === 'transfer_action') {
          await new Promise(resolve => setTimeout(resolve, 120));
          if (window.__failTransfer) throw new Error('取消共享失败测试');
          if (args.action === 'pickfile') window.__state.sharedFiles.push(...structuredClone(window.__pickedFiles));
          if (args.action === 'remove') window.__state.sharedFiles = window.__state.sharedFiles.filter(file => file.id !== args.id);
          if (args.action === 'removemany') window.__state.sharedFiles = window.__state.sharedFiles.filter(file => !args.ids.includes(file.id));
        } else if (command === 'desktop_action') {
          if (['halosmall', 'halomedium', 'halolarge'].includes(args.action)) {
            window.__haloRequests.push(args.action);
            await new Promise(resolve => setTimeout(resolve, 650));
            window.__state.halo.size = args.action.slice(4);
          }
          if (args.action?.startsWith('pairinginterval')) { window.__state.pairingIntervalMinutes = Number(args.action.slice(15)); window.__state.codeRemaining = window.__state.pairingIntervalMinutes * 60; }
          if (args.action === 'stop') window.__state.running = false;
          if (args.action === 'start') window.__state.running = true;
        } else throw new Error('unsupported simulated command');
        return structuredClone(window.__state);
      },
    };
  });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  await page.getByText('暂时无法读取电脑服务状态，正在自动重试…').waitFor();
  await page.getByRole('heading', { name: '等待手机配对', exact: true }).waitFor();
  await page.locator('.qr-card img').waitFor();
  if (process.argv.includes('--receive-transfers')) {
    checks.push(...await verifyDesktopReceive(page, output));
  } else if (process.argv.includes('--connection-states')) {
    checks.push(...await verifyDesktopConnections(page, output));
  } else {
  assert.equal(await page.locator('.pair-code').innerText(), '012345');
  const initialQr = await page.locator('.qr-card img').getAttribute('src');
  const qrBox = await page.locator('.qr-frame').boundingBox();
  await page.evaluate(() => { window.__state.code = '654321'; window.__state.codeRemaining = 600; });
  await page.waitForFunction(old => document.querySelector('.qr-card img')?.src !== old, initialQr);
  await page.locator('.qr-card img').waitFor();
  assert.deepEqual(await page.locator('.qr-frame').boundingBox(), qrBox);
  const selected = await page.evaluate(() => window.__state.selected);
  const payloadQr = await QRCode.toDataURL(`plumelink://connect?address=${encodeURIComponent(`${selected}:17840`)}&code=654321`, { width:320, margin:4, errorCorrectionLevel:'M', color:{dark:'#203653',light:'#ffffff'} });
  const checkQr = expectedQr => page.waitForFunction(async expected => {
    const actual = document.querySelector('.qr-card img');
    if (!actual?.complete || !actual.naturalWidth) return false;
    const image = new Image(); image.src = expected; await image.decode();
    const pixels = img => { const canvas = document.createElement('canvas'); canvas.width = canvas.height = 320; const ctx = canvas.getContext('2d'); ctx.drawImage(img,0,0); return ctx.getImageData(0,0,320,320).data; };
    const a = pixels(actual), b = pixels(image);
    return a.every((value,index) => value === b[index]);
  }, expectedQr);
  await checkQr(payloadQr);
  await page.evaluate(() => { window.__state.codeRemaining = 0; });
  await page.locator('.qr-frame>span').waitFor();
  assert.deepEqual(await page.locator('.qr-frame').boundingBox(), qrBox);
  await page.evaluate(() => { window.__state.code = '012345'; window.__state.codeRemaining = 600; });
  await page.waitForFunction(src => document.querySelector('.qr-card img')?.src === src, initialQr);
  checks.push('QR follows pairing-code renewal with a stable reserved frame and hides expired credentials');
  checks.push('initial bridge failure recovers', 'leading-zero pairing code and QR');
  assert.equal(await page.locator('.titlebar-brand').count(), 1);
  assert.equal(await page.title(), '羽连 PlumeLink');
  assert.equal((await page.locator('.window-drag-region > span').innerText()).trim(), '羽连 PlumeLink');
  assert.equal(await page.locator('.desktop-sidebar img').count(), 0);
  assert(await page.locator('.titlebar-brand').evaluate(image => image.complete && image.naturalWidth > 0));
  await page.getByRole('button', { name: '最大化', exact: true }).click();
  await page.getByRole('button', { name: '还原窗口', exact: true }).waitFor();
  await page.getByRole('button', { name: '还原窗口', exact: true }).press('Enter');
  await page.getByRole('button', { name: '最大化', exact: true }).waitFor();
  await page.locator('.window-drag-region').dblclick();
  await page.getByRole('button', { name: '还原窗口', exact: true }).waitFor();
  await page.evaluate(() => { window.__maximized = false; dispatchEvent(new Event('resize')); });
  await page.getByRole('button', { name: '最大化', exact: true }).waitFor();
  await page.getByRole('button', { name: '最小化', exact: true }).click();
  await page.getByRole('button', { name: '关闭窗口', exact: true }).click();
  const windowCalls = await page.evaluate(() => window.__windowCalls);
  assert(windowCalls.includes('start_dragging'));
  assert(windowCalls.includes('hide'));
  assert(windowCalls.includes('close'));
  assert.equal(windowCalls.filter(action => action === 'toggle_maximize').length, 3);
  await page.evaluate(() => window.__failWindow = true);
  await page.getByRole('button', { name: '最大化', exact: true }).click();
  await page.getByText('窗口操作失败，请稍后重试', { exact: true }).waitFor();
  assert(await page.getByRole('button', { name: '最大化', exact: true }).isEnabled());
  await page.getByRole('button', { name: '关闭操作提示' }).click();
  await page.evaluate(() => window.__failWindow = false);
  await page.evaluate(() => window.__stallWindow = true);
  await page.getByRole('button', { name: '最大化', exact: true }).click();
  await page.getByText('窗口操作失败，请稍后重试', { exact: true }).waitFor();
  assert(await page.getByRole('button', { name: '最大化', exact: true }).isEnabled());
  await page.evaluate(() => window.__stallWindow = false);
  await page.getByRole('button', { name: '关闭操作提示' }).click();
  await page.getByRole('button', { name: '最大化', exact: true }).click();
  await page.getByRole('button', { name: '还原窗口', exact: true }).waitFor();
  await page.getByRole('button', { name: '还原窗口', exact: true }).click();
  await page.getByRole('button', { name: '最大化', exact: true }).waitFor();
  checks.push('stalled window command times out and controls recover for retry');
  checks.push('custom titlebar: supplied logo only at the top, drag/double-click, keyboard maximize/restore, external resize synchronization, minimize/close and recoverable failures');
  await page.getByRole('button', { name: '复制地址', exact: true }).scrollIntoViewIfNeeded();
  const addressBefore = await page.locator('.connection-address').boundingBox();
  const footerBeforeCopy = await page.locator('footer').boundingBox();
  await page.getByRole('button', { name: '复制地址', exact: true }).click();
  await page.getByText('地址已复制', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.__clipboard), 'https://192.168.1.20:17840');
  assert.deepEqual(await page.locator('.connection-address').boundingBox(), addressBefore);
  assert.deepEqual(await page.locator('footer').boundingBox(), footerBeforeCopy);
  await page.getByRole('button', { name: '关闭操作提示' }).click();
  assert.equal(await page.locator('.feedback-toast').count(), 0);
  checks.push('copy feedback is dismissible without changing the address or footer layout');
  for (const colorScheme of ['light', 'dark']) {
    await page.emulateMedia({ colorScheme, reducedMotion: 'reduce' });
    for (const width of [480, 760, 1200]) {
      await page.setViewportSize({ width, height: 760 });
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      await page.screenshot({ path: resolve(output, `${colorScheme}-${width}.png`), fullPage: true });
    }
  }
  checks.push('light and dark layouts at 480/760/1200px');
  await page.emulateMedia({ colorScheme: 'light' });
  await page.setViewportSize({ width: 760, height: 760 });
  await page.evaluate(() => Object.assign(window.__state, {
    trusted: true, pairedCount: 1, connected: true, connectedCount: 1,
    devices: [{ id: 'test', name: '客厅的 Android 手机', online: true, pairedAt: 0, lastSeen: 0,
      permissions: { admin: true, input: true, text: true, files: true, microphone: true, power: false } }],
  }));
  await page.getByRole('heading', { name: '1 台设备在线' }).waitFor();
  await page.getByRole('button', { name: /^设备管理/ }).click();
  assert(await page.locator('[data-view=devices]').isVisible());
  assert(!(await page.locator('[data-view=overview]').isVisible()));
  const adminPermission = page.getByRole('checkbox', { name: '管理员控制', exact: true });
  assert(await adminPermission.isChecked());
  await page.evaluate(() => { window.__state.devices[0].online = false; });
  await page.getByText(/最近连接/).first().waitFor();
  await adminPermission.uncheck();
  await page.waitForFunction(() => !window.__state.devices[0].permissions.admin);
  assert(await page.getByRole('checkbox', { name: '遥控', exact: true }).isChecked());
  assert(await page.getByRole('checkbox', { name: '文字', exact: true }).isChecked());
  await adminPermission.check();
  await page.waitForFunction(() => window.__state.devices[0].permissions.admin);
  await page.evaluate(() => { window.__state.devices[0].online = true; });
  checks.push('administrator permission can be changed for an offline device independently of input and text');
  await page.getByRole('button', { name: '重命名 客厅的 Android 手机' }).click();
  assert(await page.getByRole('textbox', { name: '设备名称' }).evaluate(element => document.activeElement === element && element.selectionEnd === element.value.length && element.selectionStart === 0));
  await page.getByRole('textbox', { name: '设备名称' }).fill('放弃的名称');
  await page.getByRole('button', { name: '取消', exact: true }).click();
  assert(await page.getByRole('button', { name: '重命名 客厅的 Android 手机' }).evaluate(element => document.activeElement === element));
  await page.getByRole('button', { name: '重命名 客厅的 Android 手机' }).click();
  await page.getByRole('textbox', { name: '设备名称' }).press('Escape');
  assert.equal(await page.getByRole('textbox', { name: '设备名称' }).count(), 0);
  await page.getByRole('button', { name: '重命名 客厅的 Android 手机' }).click();
  await page.getByRole('textbox', { name: '设备名称' }).fill('书房平板');
  await page.getByRole('button', { name: '保存', exact: true }).click();
  await page.getByText('保存失败测试', { exact: true }).waitFor();
  assert.equal(await page.getByText('保存失败测试', { exact: true }).evaluate(element => getComputedStyle(element.closest('.error')).borderLeftWidth), '0px');
  checks.push('error feedback has no accent border');
  // Wait across two background polls: the failed edit and error must survive.
  await page.waitForTimeout(2200);
  assert.equal(await page.getByRole('textbox', { name: '设备名称' }).inputValue(), '书房平板');
  assert(await page.getByText('保存失败测试', { exact: true }).isVisible());
  await page.evaluate(() => window.__failRename = false);
  await page.getByRole('button', { name: '保存', exact: true }).click();
  await page.getByRole('button', { name: '重命名 书房平板' }).waitFor();
  checks.push('rename focuses and selects the draft; Cancel and Escape restore the trigger', 'failed rename preserves draft and error across polling', 'retry recovers');
  await page.screenshot({ path: resolve(output, 'devices.png'), fullPage: true });
  await page.getByRole('button', { name: '偏好设置', exact: true }).click();
  assert.equal(await page.locator('#pairing-interval').inputValue(), '15');
  const portInput = page.getByLabel('服务端口', {exact: true});
  const applyPort = page.getByRole('button', {name: '应用', exact: true});
  for (const invalid of ['', '0', '65536', '12.5', 'abc']) {
    await portInput.fill(invalid);
    assert(await applyPort.isDisabled());
  }
  await portInput.fill('19828');
  await applyPort.click();
  await page.locator('#port-feedback').filter({hasText:'端口 19828 已被占用'}).waitFor();
  assert.equal(await page.evaluate(() => window.__state.port), 17840);
  await portInput.fill('29827');
  await portInput.press('Enter');
  await page.waitForFunction(() => window.__state.port === 29827);
  await page.getByRole('button', { name: '总览', exact: true }).click();
  await page.getByRole('button', { name: '扫码与配对信息', exact: true }).click();
  const customQr = await QRCode.toDataURL('plumelink://connect?address=' + encodeURIComponent(selected + ':29827') + '&code=012345', {width:320, margin:4, errorCorrectionLevel:'M', color:{dark:'#203653',light:'#ffffff'}});
  await checkQr(customQr);
  await page.getByRole('button', { name: '扫码与配对信息', exact: true }).click();
  await page.getByRole('button', { name: '偏好设置', exact: true }).click();
  await page.getByRole('button', {name:'恢复默认端口（17840）'}).click();
  await applyPort.click();
  await page.waitForFunction(() => window.__state.port === 17840);
  checks.push('service port validation, occupied-port recovery, updated QR and restore default');
  await page.getByText('电脑快捷入口', { exact: true }).click();
  await page.evaluate(() => window.__failApplicationAdd = true);
  await page.getByRole('button', { name: '添加应用', exact: true }).click();
  const appPanel = page.locator('.application-settings');
  await appPanel.getByRole('alert').filter({ hasText: 'Add failed in test' }).waitFor();
  await page.evaluate(() => window.__failApplicationAdd = false);
  await appPanel.getByRole('button', { name: '重试', exact: true }).click();
  const removeApplication = page.getByRole('button', { name: '移除 Sample App', exact: true });
  await expect(removeApplication).toBeVisible({ timeout: 2000 });
  assert.equal(await page.evaluate(() => window.__applications.length), 1, 'retry must repeat the failed add, not silently read the list');
  await page.locator('.application-settings').screenshot({ path: resolve(output, 'application-shortcuts.png') });
  await appPanel.getByRole('button', { name: '重命名 Sample App', exact: true }).click();
  const appName = appPanel.getByRole('textbox', { name: '名称', exact: true });
  await appName.waitFor();
  assert.ok(await appName.evaluate(node => node === document.activeElement));
  await appName.fill('   ');
  await appPanel.getByRole('button', { name: '保存', exact: true }).click();
  await appPanel.getByRole('alert').filter({ hasText: '快捷入口名称无效' }).waitFor();
  assert.equal(await appName.getAttribute('aria-invalid'), 'true');
  assert.ok(await appName.evaluate(node => node.closest('form')?.contains(document.getElementById(node.getAttribute('aria-describedby')))), 'validation feedback belongs beside the editor');
  assert.equal(await page.evaluate(() => window.__applications[0].name), 'Sample App');
  const renameCount = await page.evaluate(() => window.__applicationCalls.filter(call => call.action === 'rename').length);
  await appName.fill('invalid\u0085name');
  await appPanel.getByRole('button', { name: '保存', exact: true }).click();
  await appPanel.getByRole('alert').filter({ hasText: '快捷入口名称无效' }).waitFor();
  assert.equal(await page.evaluate(() => window.__applicationCalls.filter(call => call.action === 'rename').length), renameCount, 'C1 control characters must not be sent for saving');
  await appPanel.getByRole('button', { name: '取消', exact: true }).click();
  await appPanel.getByRole('button', { name: '重命名 Sample App', exact: true }).click();
  await appName.fill('常用工具');
  await page.evaluate(() => window.__failApplicationRename = true);
  await appPanel.getByRole('button', { name: '保存', exact: true }).click();
  await appPanel.getByRole('alert').filter({ hasText: 'Rename failed in test' }).waitFor();
  assert.equal(await appName.inputValue(), '常用工具', 'failed save retains the draft');
  assert.ok(await appName.evaluate(node => node === document.activeElement), 'failed rename restores focus to its editor');
  assert.equal(await appPanel.getByRole('button', { name: '重试', exact: true }).count(), 0, 'rename uses the existing Save button instead of an unrelated list retry');
  await page.evaluate(() => window.__failApplicationRename = false);
  const appViewport = page.viewportSize();
  await page.setViewportSize({ width: 340, height: 720 });
  await appName.scrollIntoViewIfNeeded();
  assert.ok(await appName.evaluate(node => {
    const bounds = node.getBoundingClientRect();
    return bounds.left >= 0 && bounds.right <= innerWidth && node.scrollWidth <= node.clientWidth;
  }));
  await appPanel.screenshot({ path: resolve(output, 'application-rename-narrow.png') });
  await appPanel.getByRole('button', { name: '保存', exact: true }).click();
  const renamedButton = appPanel.getByRole('button', { name: '重命名 常用工具', exact: true });
  await renamedButton.waitFor();
  assert.ok(await renamedButton.evaluate(node => node === document.activeElement));
  assert.equal(await page.evaluate(() => window.__applications[0].path), 'Applications/Sample App.exe');
  assert.equal(await page.evaluate(() => window.__applications[0].id), 'a'.repeat(32));
  await renamedButton.click();
  await appName.fill('cancelled');
  await appName.press('Escape');
  assert.equal(await page.evaluate(() => window.__applications[0].name), '常用工具');
  for (const [language, field, cancel] of [
    ['en', 'Name', 'Cancel'], ['zh-TW', '名稱', '取消'], ['zh-CN', '名称', '取消'],
  ]) {
    await page.locator('#desktop-language').selectOption(language);
    await page.locator('#rename-app-' + 'a'.repeat(32)).click();
    const localizedInput = appPanel.getByRole('textbox', { name: field, exact: true });
    await localizedInput.scrollIntoViewIfNeeded();
    assert.ok(await localizedInput.evaluate(node => node.getBoundingClientRect().right <= innerWidth));
    await localizedInput.fill('   ');
    await appPanel.getByRole('button', { name: language === 'en' ? 'Save' : '保存', exact: true }).click();
    assert.ok(await localizedInput.evaluate(node => {
      const error = document.getElementById(node.getAttribute('aria-describedby'));
      if (!error || error.closest('form') !== node.closest('form')) return false;
      const box = error.getBoundingClientRect();
      return error.scrollWidth <= error.clientWidth && box.left >= 0 && box.right <= innerWidth;
    }), 'localized errors stay beside the input without horizontal overflow');
    await appPanel.screenshot({ path: resolve(output, 'application-rename-' + language + '.png') });
    await appPanel.getByRole('button', { name: cancel, exact: true }).click();
  }
  await appPanel.getByRole('button', { name: '移除 常用工具', exact: true }).click();
  await expect(appPanel.getByRole('button', { name: '添加应用', exact: true })).toBeFocused();
  await page.evaluate(() => window.__cancelFolderPicker = true);
  await appPanel.getByRole('button', { name: '添加文件夹', exact: true }).click();
  assert.equal(await page.evaluate(() => window.__applications.length), 0, 'canceling the folder picker leaves entries intact');
  await page.evaluate(() => window.__cancelFolderPicker = false);
  await appPanel.getByRole('button', { name: '添加文件夹', exact: true }).click();
  await appPanel.getByText('工作资料 (Q4) & Notes', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.__applications[0].kind), 'folder');
  for (const [language, add] of [['en', 'Add folder'], ['zh-TW', '新增資料夾'], ['zh-CN', '添加文件夹']]) {
    await page.locator('#desktop-language').selectOption(language);
    const addFolder = appPanel.getByRole('button', { name: add, exact: true });
    assert.ok(await addFolder.evaluate(node => { const box=node.getBoundingClientRect();return box.left>=0&&box.right<=innerWidth; }));
    assert.ok(await appPanel.locator('li').evaluate(node => {
      const copy=node.querySelector('.entry-copy').getBoundingClientRect();
      const actions=node.querySelector('.entry-actions').getBoundingClientRect();
      return copy.width>=110&&actions.top>=copy.bottom;
    }), 'narrow rows reserve space for names and place actions below the copy');
    await appPanel.screenshot({ path: resolve(output, 'folder-shortcuts-' + language + '.png') });
  }
  await appPanel.getByRole('button', { name: '重命名 工作资料 (Q4) & Notes', exact: true }).click();
  await appName.fill('项目文件');
  await appPanel.getByRole('button', { name: '保存', exact: true }).click();
  assert.equal(await page.evaluate(() => window.__applications[0].path), 'Documents/工作资料 (Q4) & Notes');
  await appPanel.getByRole('button', { name: '添加应用', exact: true }).click();
  await removeApplication.waitFor();
  await page.evaluate(() => window.__failApplicationRemove = true);
  await appPanel.getByRole('button', { name: '移除 项目文件', exact: true }).click();
  await appPanel.getByRole('alert').filter({ hasText: 'Remove failed in test' }).waitFor();
  assert.equal(await page.evaluate(() => window.__applications.length), 2, 'failed removal preserves entries');
  await page.evaluate(() => window.__failApplicationRemove = false);
  await appPanel.getByRole('button', { name: '重试', exact: true }).click();
  const remainingRename = appPanel.getByRole('button', { name: '重命名 Sample App', exact: true });
  await expect(remainingRename).toBeFocused();
  assert.equal(await page.evaluate(() => window.__applications.length), 1, 'remove retry repeats removal without changing other entries');
  await remainingRename.press('Enter');
  await appName.waitFor();
  assert.equal(await page.evaluate(() => window.__applications.length), 1, 'continued Enter activates editing instead of deleting another entry');
  await appPanel.getByRole('button', { name: '取消', exact: true }).click();
  await removeApplication.click();
  await expect(appPanel.getByRole('button', { name: '添加应用', exact: true })).toBeFocused();
  await page.setViewportSize(appViewport);
  await removeApplication.waitFor({ state: 'hidden' });
  await page.getByText('电脑快捷入口', { exact: true }).click();
  checks.push('application shortcuts add/remove and rename, real add/remove retries, inline invalid-name/save errors and retained draft, focus after deletion and three-language narrow layout');

  await page.locator('#pairing-interval').selectOption('0');
  await page.waitForFunction(() => window.__state.pairingIntervalMinutes === 0);
  await page.getByRole('button', { name: '总览', exact: true }).click();
  await page.getByRole('button', { name: '扫码与配对信息', exact: true }).click();
  await page.locator('.code-status').filter({ hasText: '手动更新' }).waitFor();
  assert.equal(await page.locator('.qr-card img').count(), 1);
  await page.getByRole('button', { name: '偏好设置', exact: true }).click();
  await page.locator('#pairing-interval').selectOption('15');
  await page.waitForFunction(() => window.__state.pairingIntervalMinutes === 15);
  // Expanding details must not change the workspace width or scroll position.
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.locator('[data-view=settings] > details[open] > summary').evaluateAll(items => items.forEach(item => item.click()));
  const beforeExpand = await page.locator('.topbar').boundingBox();
  assert(await page.evaluate(() => document.querySelector('.desktop-content').scrollHeight <= document.querySelector('.desktop-content').clientHeight));
  await page.locator('[data-view=settings] > details > summary').evaluateAll(items => items.forEach(item => item.click()));
  assert(await page.evaluate(() => document.querySelector('.desktop-content').scrollHeight > document.querySelector('.desktop-content').clientHeight));
  const afterExpand = await page.locator('.topbar').boundingBox();
  assert.equal(afterExpand.x, beforeExpand.x);
  assert.equal(afterExpand.width, beforeExpand.width);
  assert.equal(await page.locator('.desktop-content').evaluate(element => element.scrollTop), 0);
  checks.push('details expansion crosses the scrollbar threshold without changing workspace width or scroll position');

  const sizeSelect = page.locator('#halo-size');
  await sizeSelect.scrollIntoViewIfNeeded();
  await sizeSelect.selectOption('large');
  await page.waitForFunction(() => window.__haloRequests.length === 1);
  const sizeRect = await sizeSelect.boundingBox();
  await sizeSelect.selectOption('small');
  assert.equal(await sizeSelect.inputValue(), 'small');
  assert(await sizeSelect.isEnabled());
  assert(await page.getByRole('button', { name: '停止遥控', exact: true }).isEnabled());
  await page.waitForFunction(() => window.__haloRequests.length === 2);
  assert.equal(await sizeSelect.inputValue(), 'small', 'older response must not repaint the pending selection');
  const pendingRect = await sizeSelect.boundingBox();
  assert.equal(pendingRect.x, sizeRect.x);
  assert.equal(pendingRect.y, sizeRect.y);
  await page.waitForFunction(() => window.__state.halo.size === 'small');
  assert.equal(await sizeSelect.inputValue(), 'small');
  checks.push('rapid dropdown selections serialize without disabling the page, changing position, or reverting to an older response');

  await page.locator('select[aria-label="麦克风输出设备"] option[value="uu"]').waitFor({ state: 'attached' });
  assert.equal(await page.getByRole('option', { name: '扬声器 (UU远程虚拟音频设备)', exact: true }).count(), 1);
  await page.evaluate(() => window.__serviceOnly = true);
  await page.getByRole('button', { name: '刷新设备', exact: true }).click();
  await page.getByText('驱动已存在，但标准播放端不可用。请检查 Windows 声音设置，或在安装后重启再检测。').waitFor();
  assert.equal(await page.getByRole('button', { name: '安装 VB-CABLE' }).count(), 0);
  const reinstall = page.locator('.mic-reinstall-link');
  await page.mouse.move(0,0);
  for (const [language,label] of [['zh-CN','点击重新安装'],['zh-TW','點擊重新安裝'],['en','Click to reinstall']]) {
    await page.locator('#desktop-language').selectOption(language);
    await expect(reinstall).toHaveText(label);
    for (const theme of ['light','dark']) {
      await page.locator('#desktop-theme').selectOption(theme);
      for (const width of [480,1000]) {
        await page.setViewportSize({width,height:800});
        assert(await reinstall.evaluate(button=>{
          const parent=button.parentElement, a=getComputedStyle(button), b=getComputedStyle(parent);
          const reference=document.createElement('span');reference.style.color='var(--accent)';parent.append(reference);
          const accent=getComputedStyle(reference).color;reference.remove();
          return a.fontSize===b.fontSize && a.fontFamily===b.fontFamily && a.lineHeight===b.lineHeight
            && a.color===accent
            && a.textDecorationLine==='none' && a.backgroundColor==='rgba(0, 0, 0, 0)'
            && button.getBoundingClientRect().height<=parent.getBoundingClientRect().height
            && parent.scrollWidth<=parent.clientWidth;
        }),`reinstall link typography or overflow in ${language}/${theme}/${width}`);
        if(theme==='light' && width===1000) await page.locator('.microphone-settings .mic-section').first().screenshot({path:resolve(output,`microphone-reinstall-${language}.png`)});
      }
    }
  }
  await page.locator('#desktop-language').selectOption('zh-CN');
  await page.locator('#desktop-theme').selectOption('system');
  await page.setViewportSize({width:760,height:760});
  await page.evaluate(()=>{window.__cancelInstaller=true;window.__holdCableInstall=true;});
  await reinstall.click();
  await page.waitForFunction(()=>typeof window.__finishCableInstall==='function');
  await expect(reinstall).toBeDisabled();
  await reinstall.evaluate(button=>button.click());
  assert.equal(await page.evaluate(()=>window.__cableInstallRequests),1);
  await page.evaluate(()=>{window.__holdCableInstall=false;window.__finishCableInstall();});
  await page.getByText('已取消管理员授权。',{exact:true}).waitFor();
  await expect(reinstall).toBeEnabled();
  await page.evaluate(() => { window.__serviceOnly = false; window.__cancelInstaller = true; });
  await page.getByRole('button', { name: '刷新设备', exact: true }).click();
  await page.getByRole('button', { name: '安装 VB-CABLE' }).click();
  await page.getByText('已取消管理员授权。', { exact: true }).waitFor();
  await page.evaluate(() => window.__cancelInstaller = false);
  await page.evaluate(() => window.__serviceOnly = true);
  await page.getByRole('button', { name: '刷新设备', exact: true }).click();
  await reinstall.focus();
  await reinstall.press('Enter');
  await page.getByText('录音端已收到测试音，请在收音软件中选择「CABLE Output (test)」。').waitFor();
  await expect(reinstall).toHaveCount(0);
  checks.push('inline VB-CABLE reinstall: matching hint typography, blue without underline, three languages, light/dark and 480/1000px; single pending request, UAC cancellation recovery and keyboard activation');
  assert.equal(await page.getByRole('button', { name: '安装 VB-CABLE' }).count(), 0);
  await page.getByRole('button', { name: '检测并配置麦克风' }).click();
  await page.waitForFunction(() => window.__audioSelection === 'cable');
  await page.getByRole('combobox', { name: '麦克风输出设备' }).selectOption('cable');
  await page.getByRole('button', { name: '播放测试音并配置' }).click();
  await page.getByText('录音端已收到测试音，请在收音软件中选择「CABLE Output (test)」。').waitFor();
  await page.getByRole('button', { name: '保存音频设备', exact: true }).click();
  await page.waitForFunction(() => window.__audioSelection === 'cable');
  const gain = page.locator('#microphone-gain');
  const changeGain = async value => {
    await gain.evaluate((input, value) => {
      input.value = String(value);
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
    }, value);
  };
  await page.evaluate(() => { window.__holdGain = true; window.__failGain = true; });
  await changeGain(24);
  await page.waitForFunction(() => typeof window.__finishGain === 'function');
  assert(await gain.isDisabled(), 'pending gain must not submit another save');
  await page.evaluate(() => window.__finishGain());
  await page.getByText('增益保存失败测试', { exact: true }).waitFor();
  assert.equal(await gain.inputValue(), '6', 'failed gain save must restore the confirmed setting');
  assert.equal(await page.locator('output[for="microphone-gain"]').innerText(), '+6 dB');
  await page.evaluate(() => { window.__holdGain = false; window.__failGain = false; });
  await changeGain(12);
  await page.waitForFunction(() => window.__gainDb === 12 && !document.querySelector('#microphone-gain').disabled);
  assert.equal(await page.getByText('增益保存失败测试', { exact: true }).count(), 0, 'successful retry must clear the old failure');

  await page.evaluate(() => { window.__holdAudioList = true; window.__finishAudioList = null; });
  await page.getByRole('button', { name: '刷新设备', exact: true }).click();
  await page.waitForFunction(() => typeof window.__finishAudioList === 'function');
  await changeGain(24);
  await page.waitForFunction(() => window.__gainDb === 24 && !document.querySelector('#microphone-gain').disabled);
  await page.evaluate(() => { window.__holdAudioList = false; window.__finishAudioList(); });
  await page.waitForFunction(() => !document.querySelector('.mic-device-actions button').disabled);
  assert.equal(await gain.inputValue(), '24', 'a refresh started before gain save must not restore the old setting');

  await page.evaluate(() => { window.__holdGain = true; window.__finishGain = null; });
  await changeGain(30);
  await page.waitForFunction(() => typeof window.__finishGain === 'function');
  await page.evaluate(() => { window.__holdAudioList = true; window.__finishAudioList = null; });
  await page.getByRole('button', { name: '刷新设备', exact: true }).click();
  await page.waitForFunction(() => typeof window.__finishAudioList === 'function');
  await page.evaluate(() => { window.__holdGain = false; window.__finishGain(); });
  await page.waitForFunction(() => window.__gainDb === 30 && !document.querySelector('#microphone-gain').disabled);
  await page.evaluate(() => { window.__holdAudioList = false; window.__finishAudioList(); });
  await page.waitForFunction(() => !document.querySelector('.mic-device-actions button').disabled);
  assert.equal(await gain.inputValue(), '30', 'a refresh started during gain save must not restore the old setting');
  checks.push('gain save failure restores the confirmed value, retry clears its error, and overlapping device refreshes cannot repaint saved gain');
  await page.evaluate(() => Object.assign(window.__state.microphone, { active: true, deviceName: '测试手机', endpointName: 'CABLE Input (test)' }));
  await page.getByText('等待音频 · 测试手机', { exact: true }).waitFor();
  await page.getByText('通道已打开，等待手机音频', { exact: true }).waitFor();
  await page.evaluate(() => Object.assign(window.__state.microphone, { receivedFrames: 50, receivedSignalFrames: 0, renderedSignalSamples: 0, inputLevel: 0, outputLevel: 0 }));
  await page.getByText('已收到音频帧，样本均为静音', { exact: true }).waitFor();
  await page.evaluate(() => Object.assign(window.__state.microphone, { receivedSignalFrames: 20, inputLevel: .2 }));
  await page.getByText('已收到声音，等待播放输出', { exact: true }).waitFor();
  await page.evaluate(() => Object.assign(window.__state.microphone, { renderedSignalSamples: 8000, outputLevel: .18 }));
  await page.getByText('声音已送往播放端', { exact: true }).waitFor();
  await page.getByText('已收到音频 · 测试手机', { exact: true }).waitFor();
  assert.equal(await page.locator('meter[aria-label="手机音频信号"]').getAttribute('value'), '0.2');
  const signalTop = await page.locator('.audio-signal').evaluate(e => e.getBoundingClientRect().top);
  await page.evaluate(() => Object.assign(window.__state.microphone, { clipping:true, gainClippedSamples:12 }));
  await expect(page.locator('#gain-help.gain-clipping')).toHaveText('增益过高，请调低。');
  assert.equal(await gain.inputValue(), '30', 'overload guidance must not silently change the selected gain');
  assert.ok(Math.abs(await page.locator('.audio-signal').evaluate(e => e.getBoundingClientRect().top) - signalTop) < 1, 'overload guidance must not shift the signal panel');
  await page.screenshot({path:resolve(output,'microphone-clipping.png'),fullPage:true});
  await page.evaluate(() => Object.assign(window.__state.microphone, { clipping:false, outputLevel:1 }));
  await expect(page.locator('#gain-help')).toHaveText('声音失真时请调低增益。');
  assert.equal(await page.locator('#gain-help.gain-clipping').count(),0,'full-scale input alone is not proof of gain clipping');
  await page.evaluate(() => window.__state.microphone.outputLevel=.18);
  await page.screenshot({ path: resolve(output, 'microphone.png'), fullPage: true });
  const longDeviceName = 'LivingRoom-Workstation-'.repeat(12);
  await page.evaluate(name => window.__state.microphone.deviceName = name, longDeviceName);
  await page.getByText(`已收到音频 · ${longDeviceName}`, {exact:true}).waitFor();
  await page.setViewportSize({width:480,height:760});
  await page.locator('.microphone-settings summary').scrollIntoViewIfNeeded();
  assert(await page.locator('.microphone-settings summary').evaluate(e=>e.scrollWidth<=e.clientWidth));
  await page.screenshot({path:resolve(output,'microphone-long-name-480.png')});
  await page.evaluate(()=>window.__state.microphone.deviceName='测试手机');
  await page.getByText('已收到音频 · 测试手机', {exact:true}).waitFor();
  checks.push('long active microphone device names wrap within the summary at minimum desktop width');
  await page.getByRole('button', { name: '连接帮助', exact: true }).click();
  await page.getByRole('button', { name: '打开诊断', exact: true }).click();
  await page.waitForSelector('#desktop-diagnostics[open] summary:focus');
  await page.getByText('麦克风阵列 (UU远程虚拟音频设备)', { exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: '检测通路', exact: true }).count(), 1);
  const outputVolume = page.locator('.mic-endpoint-volumes [data-endpoint="output"]');
  const recordingVolume = page.locator('.mic-endpoint-volumes [data-endpoint="recording"]');
  await recordingVolume.locator('dd').getByText('尚未确认', {exact:true}).waitFor();
  const refreshVolumes = async () => {
    const refresh = page.getByRole('button', { name: '刷新设备状态', exact: true });
    await refresh.click();
    await expect(refresh).toBeEnabled();
  };
  await page.evaluate(() => {
    window.__verifiedInput = window.__recordingInputs[0];
    window.__endpointVolumes.cable = { volumePercent: 12, muted: true };
    window.__endpointVolumes['uu-mic'] = { volumePercent: 70, muted: false };
  });
  await refreshVolumes();
  assert.match(await outputVolume.locator('dd').innerText(), /音量 12%/);
  assert.match(await outputVolume.locator('dd').innerText(), /已静音/);
  assert.equal(await recordingVolume.locator('dd').innerText(), '音量 70%');
  await page.getByText('请在 Windows 声音设置中取消静音或调高对应设备的音量。', { exact:true }).waitFor();
  await page.evaluate(() => { window.__endpointVolumes.cable = {volumePercent:100,muted:false}; window.__endpointVolumes['uu-mic'] = {volumePercent:0,muted:false}; });
  await refreshVolumes();
  assert.equal(await recordingVolume.locator('dd').innerText(), '音量 0%');
  assert(await recordingVolume.locator('dd').evaluate(node => node.classList.contains('is-muted')));
  await page.evaluate(() => { window.__endpointVolumes['uu-mic'] = {volumePercent:null,muted:null,error:'无法读取设备音量，请刷新设备状态。'}; });
  await refreshVolumes();
  assert.equal(await recordingVolume.locator('dd').innerText(), '无法读取');
  assert.equal(await recordingVolume.locator('.is-muted').count(), 0, 'unknown metadata must not look like mute or zero volume');
  await page.evaluate(() => { window.__failLevels = true; });
  await refreshVolumes();
  assert.equal(await outputVolume.locator('dd').innerText(), '无法读取');
  await page.getByText('读取设备音量超时，请稍后刷新。', {exact:true}).waitFor();
  await page.evaluate(() => { window.__failLevels = false; window.__endpointVolumes = {}; });
  await refreshVolumes();
  assert.equal(await outputVolume.locator('dd').innerText(), '音量 100%');
  assert.equal(await page.getByText('读取设备音量超时，请稍后刷新。', {exact:true}).count(), 0);
  await page.evaluate(() => { window.__endpointVolumes.cable = {id:'previous-output',volumePercent:0,muted:true}; });
  await refreshVolumes();
  assert.equal(await outputVolume.locator('dd').innerText(), '无法读取', 'a different endpoint snapshot must not describe the selected device');
  await page.evaluate(() => { window.__endpointVolumes.cable = {volumePercent:undefined,muted:undefined}; });
  await refreshVolumes();
  assert.equal(await outputVolume.locator('dd').innerText(), '无法读取', 'missing metadata must not show undefined percentages');
  await page.evaluate(() => { window.__endpointVolumes = {}; });
  await refreshVolumes();
  checks.push('selected playback/verified recording volume and mute are distinct; unknown, timeout, missing fields and wrong-endpoint responses never appear as healthy or zero volume');
  await page.evaluate(() => window.__recordingInputs = []);
  await page.getByRole('button', { name: '刷新设备状态', exact: true }).click();
  await page.getByText('未发现可用录音设备。请安装并启用虚拟音频线缆，再刷新设备。', { exact: true }).waitFor();
  await page.screenshot({ path: resolve(output, 'microphone-diagnostics.png'), fullPage: true });
  await page.setViewportSize({ width: 480, height: 760 });
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  checks.push('audio opening, silent packets, signal reception and submitted output are distinguished', 'recording devices listed without inferring loopback from virtual names');
  await page.getByRole('button', { name: '偏好设置', exact: true }).click();
  await page.evaluate(() => { window.__holdAudioList = true; window.__finishAudioList = null; });
  await page.getByRole('button', { name: '刷新设备', exact: true }).click();
  await page.waitForFunction(() => typeof window.__finishAudioList === 'function');
  assert(await page.getByRole('button', { name: '停止接收', exact: true }).isEnabled(), 'a stalled device refresh must not disable stop');
  await page.evaluate(() => { window.__failAudioStop = true; });
  await page.getByRole('button', { name: '停止接收', exact: true }).click();
  await page.getByText('停止接收失败测试', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.__state.microphone.active), true, 'failed stop must not pretend reception stopped');
  assert(await page.getByRole('button', { name: '停止接收', exact: true }).isEnabled(), 'failed stop can be retried while device refresh is pending');
  await page.evaluate(() => { window.__failAudioStop = false; });
  await page.getByRole('button', { name: '停止接收', exact: true }).click();
  await page.waitForFunction(() => !window.__state.microphone.active);
  await page.waitForFunction(() => !document.querySelector('.audio-signal'));
  await page.evaluate(() => { window.__holdAudioList = false; window.__finishAudioList(); });
  await page.waitForFunction(() => !document.querySelector('.mic-device-actions button').disabled);
  assert.equal(await page.locator('.microphone-settings .mic-section:not(.mic-controls) > p.hint[role="status"]').count(), 0, 'stopping does not require a device-list response');
  checks.push('stop reception remains available during blocked device enumeration and accepts its independent result');
  checks.push('virtual microphone output selection and explicit stop');
  await page.getByRole('button', { name: '总览', exact: true }).click();
  await page.getByRole('button', { name: '停止遥控', exact: true }).click();
  await page.getByRole('heading', { name: '遥控已停止', exact: true }).waitFor();
  await page.getByRole('button', { name: '启动遥控', exact: true }).click();
  await page.getByRole('heading', { name: '1 台设备在线', exact: true }).waitFor();
  for (const colorScheme of ['light', 'dark']) {
    await page.emulateMedia({ colorScheme });
    for (const width of [480, 760, 1200]) {
      await page.setViewportSize({ width, height: 780 });
      for (const [name, view] of [['设备管理', 'devices'], ['文件传输', 'transfer'], ['偏好设置', 'settings'], ['连接帮助', 'diagnostics'], ['总览', 'overview']]) {
        await page.getByRole('button', { name: new RegExp('^' + name) }).first().click();
        assert(await page.locator(`[data-view=${view}]`).isVisible());
        assert.equal(await page.locator('.workspace-view:visible').count(), 1);
        assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${view} overflow at ${width}`);
        assert.equal((await page.locator('.window-titlebar').boundingBox()).y, 0);
        assert(await page.getByRole('button', { name: '关闭窗口', exact: true }).evaluate(element => { const rect = element.getBoundingClientRect(); return rect.right <= innerWidth && rect.top >= 0 && rect.bottom <= innerHeight; }));
        if (view === 'transfer') assert(await page.locator('.transfer-intro h3').evaluate(element => element.getBoundingClientRect().height < 50), `transfer heading wraps excessively at ${width}`);
        if (view === 'settings') {
          const aligned = await page.locator('.preference-row:not(.network-row) select').evaluateAll(selects =>
            selects.every(select => {
              const field = select.getBoundingClientRect();
              const label = select.labels[0].getBoundingClientRect();
              return field.height >= 36 && field.left - label.right >= 12
                && Math.abs((field.top + field.bottom - label.top - label.bottom) / 2) <= 1;
            }));
          assert(aligned, `preference labels and controls overlap or are misaligned at ${width}`);
          const network = page.locator('#network');
          assert.equal(await network.getAttribute('aria-describedby'), 'network-help');
          const field = await network.boundingBox(), help = await page.locator('#network-help').boundingBox();
          assert(Math.abs(help.x - field.x) <= 1 && help.y >= field.y + field.height + 4);
        }
        await page.evaluate(() => scrollTo(0, 0));
        await page.screenshot({ path: resolve(output, `${view}-${colorScheme}-${width}.png`), fullPage: true });
      }
    }
  }
  checks.push('all five workspaces switch independently at 480/760/1200 in light and dark modes');
  await page.emulateMedia({ colorScheme: 'light' });
  await page.setViewportSize({ width: 1200, height: 780 });
  await page.getByRole('button', { name: '文件传输', exact: true }).click();
  assert.equal(await page.getByRole('button', { name: /^选择文件/ }).count(), 1);
  assert(await page.locator('.transfer-intro').evaluate(element => element.getBoundingClientRect().height < 150));
  const fileViewport = page.locator('.shared-file-viewport');
  await fileViewport.scrollIntoViewIfNeeded();
  const emptyHeight = await fileViewport.evaluate(element => element.getBoundingClientRect().height);
  await page.evaluate(() => window.__pickedFiles = Array.from({ length: 18 }, (_, index) => ({ id: index + 1, name: `文件-${index + 1}.txt`, size: 1024 })));
  await page.getByRole('button', { name: '选择文件', exact: true }).click();
  await page.waitForFunction(() => window.__state.sharedFiles.length === 18);
  await page.locator('.shared-files li').last().waitFor({ state: 'attached' });
  assert.equal(await page.locator('.shared-files li').count(), 18);
  const populatedHeight = await fileViewport.evaluate(element => element.getBoundingClientRect().height);
  assert(populatedHeight >= emptyHeight, 'empty lists stay compact while populated lists keep a stable scroll area');
  const fileSearch = page.getByRole('searchbox', { name: '搜索文件名', exact: true });
  const browseHeights = await page.locator('.file-browse-tools').evaluate(element => [...element.querySelectorAll('input, select')].map(control => control.getBoundingClientRect().height));
  assert(Math.abs(browseHeights[0] - browseHeights[1]) <= 1, 'search and sort controls have matching heights');
  await fileSearch.fill('文件-18');
  await page.waitForFunction(() => document.querySelectorAll('.shared-files li').length === 1);
  await page.getByRole('checkbox', { name: /^全选/ }).check();
  assert.equal(await page.locator('.shared-files li.selected').count(), 1);
  await fileSearch.fill('no-matching-file');
  await page.waitForFunction(() => document.querySelectorAll('.shared-files li.selected').length === 0);
  await page.getByText('没有匹配的文件', { exact: true }).waitFor();
  await fileSearch.fill('');
  await page.waitForFunction(() => document.querySelectorAll('.shared-files li').length === 18);
  await page.getByRole('combobox', { name: '排序方式' }).selectOption('name');
  assert.equal(await page.locator('.shared-files li b').nth(1).textContent(), '文件-2.txt');
  await page.getByRole('combobox', { name: '排序方式' }).selectOption('default');
  await page.evaluate(() => window.__state.sharedFiles[17].name = '很长的文件名称'.repeat(40) + '.txt');
  await page.locator('.shared-files li').last().locator('b[title]').waitFor();
  for (const colorScheme of ['light', 'dark']) {
    await page.emulateMedia({ colorScheme });
    for (const width of [480, 760, 1200]) {
      await page.setViewportSize({ width, height: 780 });
      await page.getByRole('checkbox', { name: /^全选/ }).uncheck();
      if (width === 1200) {
        await page.locator('.shared-files li').first().hover();
        await page.screenshot({ path: resolve(output, `files-hover-${colorScheme}.png`), fullPage: true });
      }
      const unselectedHeight = await page.locator('.file-list-toolbar').evaluate(element => element.getBoundingClientRect().height);
      await page.getByRole('checkbox', { name: /^全选/ }).check();
      assert.equal(await page.locator('.shared-files li.selected').count(), 18);
      assert.equal(await page.locator('.file-list-toolbar').evaluate(element => element.getBoundingClientRect().height), unselectedHeight);
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      assert(await fileViewport.evaluate(element => element.scrollWidth <= element.clientWidth));
      assert(await page.locator('.transfer-intro h3').evaluate(element => element.getBoundingClientRect().height < 50));
      await page.evaluate(() => scrollTo(0, 0));
      await page.screenshot({ path: resolve(output, `files-populated-${colorScheme}-${width}.png`), fullPage: true });
    }
  }
  await page.getByRole('checkbox', { name: /^全选/ }).uncheck();
  await page.emulateMedia({ colorScheme: 'light' });
  await page.setViewportSize({ width: 1200, height: 780 });
  await fileViewport.scrollIntoViewIfNeeded();
  await fileViewport.evaluate(element => element.scrollTop = 0);
  const toolbarBefore = await page.locator('.file-list-toolbar').boundingBox();
  const scrollBefore = await page.locator('.desktop-content').evaluate(element => element.scrollTop);
  const firstCancel = page.getByRole('button', { name: '取消共享 文件-1.txt', exact: true });
  const cancelBefore = await firstCancel.boundingBox();
  await firstCancel.click();
  await page.waitForFunction(() => window.__state.sharedFiles.length === 17);
  await firstCancel.waitFor({ state: 'detached' });
  const cancelAfter = await page.getByRole('button', { name: '取消共享 文件-2.txt', exact: true }).boundingBox();
  assert.equal(cancelAfter.x, cancelBefore.x);
  assert.equal(cancelAfter.y, cancelBefore.y);
  assert.equal(await page.locator('.desktop-content').evaluate(element => element.scrollTop), scrollBefore);
  assert.equal((await page.locator('.file-list-toolbar').boundingBox()).y, toolbarBefore.y);
  await page.getByRole('checkbox', { name: '选择 文件-2.txt', exact: true }).check();
  await page.getByRole('checkbox', { name: '选择 文件-3.txt', exact: true }).check();
  await page.evaluate(() => window.__failTransfer = true);
  await page.getByRole('button', { name: /^取消所选共享/ }).click();
  await page.getByText('取消共享失败测试', { exact: true }).waitFor();
  assert(await page.getByRole('checkbox', { name: '选择 文件-2.txt', exact: true }).isChecked());
  assert.equal(await page.locator('.shared-files li').count(), 17);
  await page.evaluate(() => window.__failTransfer = false);
  await page.getByRole('button', { name: '关闭错误提示', exact: true }).click();
  await page.getByRole('button', { name: /^取消所选共享/ }).click();
  await page.waitForFunction(() => window.__state.sharedFiles.length === 15);
  assert.equal(await page.getByRole('checkbox', { name: '选择 文件-2.txt', exact: true }).count(), 0);
  await page.getByRole('checkbox', { name: /^全选/ }).check();
  assert.equal(await page.locator('.shared-files input:checked').count(), 15);
  await page.getByRole('button', { name: '全部取消共享', exact: true }).click();
  await page.getByText('暂无共享文件，选择后会显示在这里。', { exact: true }).waitFor();
  assert.equal(await fileViewport.evaluate(element => element.getBoundingClientRect().height), emptyHeight);
  assert.equal(await page.locator('.file-list-toolbar').count(), 0, 'empty lists hide unavailable bulk actions');
  assert.equal(await page.getByRole('button', { name: '全部取消共享', exact: true }).count(), 0);
  checks.push('multi-file selection, fixed list viewport and stationary repeat-remove target', 'batch cancellation preserves selections on failure and retries; select all and clear keep the toolbar stationary');
  await page.screenshot({ path: resolve(output, 'file-list-stability.png'), fullPage: true });
  // The workspace owns the only page scrollbar, including at minimum window size.
  for (const [width, height] of [[480, 420], [760, 560], [1120, 780]]) {
    await page.setViewportSize({ width, height });
    await page.getByRole('button', { name: '偏好设置', exact: true }).click();
    const titlebarBefore = await page.locator('.window-titlebar').boundingBox();
    const sidebarBefore = await page.locator('.desktop-sidebar').boundingBox();
    assert.equal(titlebarBefore.width, width);
    const geometry = await page.locator('.desktop-content').evaluate(element => {
      const rect = element.getBoundingClientRect();
      element.scrollTop = element.scrollHeight;
      return { top: rect.top, bottom: rect.bottom, scrollTop: element.scrollTop, overflow: element.scrollWidth > element.clientWidth };
    });
    assert.equal(geometry.top, titlebarBefore.height);
    assert.equal(geometry.bottom, height);
    assert(geometry.scrollTop > 0);
    assert(!geometry.overflow);
    assert.deepEqual(await page.locator('.window-titlebar').boundingBox(), titlebarBefore);
    assert.deepEqual(await page.locator('.desktop-sidebar').boundingBox(), sidebarBefore);
    assert(await page.evaluate(() => scrollY === 0 && document.documentElement.scrollHeight <= innerHeight));
    await page.getByRole('button', { name: '总览', exact: true }).click();
    assert.equal(await page.locator('.desktop-content').evaluate(element => element.scrollTop), 0);
    await page.screenshot({ path: resolve(output, `isolated-scroll-${width}.png`) });
  }
  checks.push('content scrollbar starts below titlebar; titlebar and sidebar stay fixed; workspace navigation resets internal scroll at 480x420/760x560/1120x780');
  for (const scale of [1.25, 1.5, 2]) {
    const scaledContext = await browser.newContext({ locale: "zh-CN", viewport: { width: 760, height: 560 }, deviceScaleFactor: scale });
    await scaledContext.addInitScript(() => {
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
        async invoke(command) {
          if (command === 'plugin:window|is_maximized') return false;
          throw new Error('scaled rendering fixture');
        },
      };
    });
    const scaledPage = await scaledContext.newPage();
    try {
      await scaledPage.goto(`http://127.0.0.1:${server.address().port}`);
      await scaledPage.locator('.titlebar-brand').waitFor();
      await scaledPage.evaluate(() => document.fonts.ready);
      assert(await scaledPage.evaluate(() => {
        const title = document.querySelector('.window-titlebar').getBoundingClientRect();
        const content = document.querySelector('.desktop-content').getBoundingClientRect();
        const close = document.querySelector('.window-close').getBoundingClientRect();
        return title.width === innerWidth && content.top === title.bottom && content.bottom === innerHeight && close.right <= innerWidth && document.documentElement.scrollHeight <= innerHeight;
      }));
      await scaledPage.screenshot({ path: resolve(output, `titlebar-scale-${scale}.png`) });
    } finally { await scaledContext.close(); }
  }
  checks.push('125/150/200 percent device scale rendering keeps titlebar controls and internal scroll bounds intact (not a system DPI transition test)');
  await page.setViewportSize({ width: 480, height: 420 });
  await page.getByRole('button', { name: '偏好设置', exact: true }).click();
  await page.evaluate(() => {
    window.__recordingInputs = [{ id:'volume-mic', name:'Virtual recording endpoint — '.repeat(12) }];
    window.__verifiedInput = window.__recordingInputs[0];
    window.__endpointVolumes = { cable:{volumePercent:12,muted:true}, 'volume-mic':{volumePercent:null,muted:null,error:'无法读取设备音量，请刷新设备状态。'} };
  });
  for (const [language, heading] of [['en', 'Preferences'], ['zh-TW', '偏好設定'], ['zh-CN', '偏好设置']]) {
    await page.locator('#desktop-language').selectOption(language);
    await page.getByRole('heading', { name: heading, exact: true }).waitFor();
    assert.equal(await page.locator('html').getAttribute('lang'), language);
    await page.locator('#desktop-language').focus();
    await page.keyboard.press('Tab');
    assert(await page.locator('#desktop-theme').evaluate(el => el === document.activeElement));
    const policy = page.locator('details').filter({ has: page.locator('.privacy-copy') });
    if (!await policy.evaluate(details => details.open)) await policy.locator('summary').click();
    await policy.locator('article').waitFor({ state: 'visible' });
    const sections = policy.locator('h3');
    assert.equal(await sections.count(), 4);
    assert.equal(await sections.first().innerText(), { en: 'Connection and remote control', 'zh-TW': '連線與遙控', 'zh-CN': '连接与遥控' }[language]);
    assert((await policy.locator('article').innerText()).includes('kuiwaiwai'));
    await policy.locator('summary').click();
    assert(await page.locator('.desktop-content').evaluate(el => el.scrollWidth <= el.clientWidth));
    const dictionary = language === 'zh-CN' ? {} : JSON.parse(await readFile(`shared/locales/${language === 'en' ? 'en' : 'zh-Hant'}.json`, 'utf8'));
    const word = key => dictionary[key] ?? key;
    await page.getByRole('button', {name:word('连接帮助'),exact:true}).click();
    const refresh = page.getByRole('button', {name:word('刷新设备状态'),exact:true});
    await refresh.click(); await expect(refresh).toBeEnabled();
    const section = page.locator('.mic-endpoint-volumes');
    await section.scrollIntoViewIfNeeded();
    assert.equal(await section.getAttribute('aria-label'), word('Windows 设备音量'));
    assert((await outputVolume.locator('dd').innerText()).includes(language === 'en' ? 'Muted' : language === 'zh-TW' ? '已靜音' : '已静音'));
    assert.equal(await recordingVolume.locator('dd').innerText(), word('无法读取'));
    assert(await section.evaluate(element => element.scrollWidth <= element.clientWidth), `endpoint state overflow in ${language}`);
    assert(await recordingVolume.locator('dt span').evaluate(element => {
      const line = parseFloat(getComputedStyle(element).lineHeight);
      return element.clientHeight <= line * 2 + 1 && element.title === element.textContent;
    }), `long endpoint names must remain compact and expose their full name in ${language}`);
    await section.screenshot({path:resolve(output,`microphone-levels-${language}.png`)});
    await page.getByRole('button', {name:heading,exact:true}).click();
    const mic = page.locator('.microphone-settings');
    if (!await mic.evaluate(element => element.open)) await mic.locator('summary').click();
    await page.evaluate(() => Object.assign(window.__state.microphone,{active:true,clipping:false}));
    await expect(page.locator('#gain-help')).toHaveText(word('声音失真时请调低增益。'));
    const normalHintHeight=await page.locator('#gain-help').evaluate(element=>element.getBoundingClientRect().height);
    await page.evaluate(() => window.__state.microphone.clipping=true);
    await expect(page.locator('#gain-help.gain-clipping')).toHaveText(word('增益过高，请调低。'));
    assert.equal(await page.locator('#gain-help').evaluate(element=>element.getBoundingClientRect().height),normalHintHeight,`gain warning should retain layout in ${language}`);
    assert(await mic.evaluate(element=>element.scrollWidth<=element.clientWidth));
    await mic.screenshot({path:resolve(output,`microphone-clipping-${language}.png`)});
    await page.evaluate(() => Object.assign(window.__state.microphone,{active:false,clipping:true}));
    await expect(page.locator('#gain-help')).toHaveText(word('声音失真时请调低增益。'));
  }
  await page.locator('#desktop-language').selectOption('en');
  await page.reload();
  await page.getByRole('button', { name: 'Preferences', exact: true }).click();
  assert.equal(await page.locator('#desktop-language').inputValue(), 'en');
  await page.locator('#desktop-language').selectOption('system');
  await page.getByRole('heading', { name: '偏好设置', exact: true }).waitFor();
  checks.push('three live interface languages at minimum window size; preference persists after reload; system default restored');
  await page.locator('#desktop-theme').selectOption('dark');
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'dark');
  await page.reload();
  await page.getByRole('button', { name: '偏好设置', exact: true }).click();
  assert.equal(await page.locator('#desktop-theme').inputValue(), 'dark');
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'dark');
  await page.locator('#desktop-theme').selectOption('light');
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'light');
  await page.locator('#desktop-theme').selectOption('system');
  assert.equal(await page.locator('html').getAttribute('data-theme'), null);
  assert(await page.locator('.desktop-content').evaluate(el => el.scrollWidth <= el.clientWidth));
  checks.push('desktop theme switches between dark, light and system; choice persists after reload');
  await page.emulateMedia({ forcedColors: 'active' });
  await page.screenshot({ path: resolve(output, 'high-contrast.png'), fullPage: true });
  checks.push('start/stop rendering', 'high contrast rendering');
  await page.emulateMedia({ forcedColors: 'none' });
  await page.getByRole('button', { name: /^(偏好设置|偏好設定|Preferences)$/ }).click();
  for (const language of ['zh-CN', 'zh-TW', 'en']) {
    await page.locator('#desktop-language').selectOption(language);
    await page.evaluate(() => {
      window.__usb.devices = [
        {id:'a'.repeat(32),name:'Fixture phone',state:'unauthorized',linked:false,selected:false,error:''},
        {id:'b'.repeat(32),name:'Offline fixture',state:'offline',linked:false,selected:false,error:''},
        {id:'c'.repeat(32),name:'Fixture Android phone with a longer device name for layout checking',state:'ready',linked:false,selected:false,error:''},
      ];
      document.querySelector('#usb-connection').open = true;
    });
    await page.locator('.usb-tools > button').first().click();
    await expect(page.locator('.usb-devices li')).toHaveCount(3);
    assert(await page.locator('.usb-devices li').nth(0).locator('button').isDisabled());
    assert(await page.locator('.usb-devices li').nth(1).locator('button').isDisabled());
    await page.locator('.usb-devices li').nth(2).locator('button').click();
    await expect(page.locator('.usb-pairing strong')).toHaveText('246802');
    await page.locator('.usb-pairing details').evaluate(details => details.open = true);
    assert(await page.locator('.usb-certificate summary > svg').first().evaluate(icon => getComputedStyle(icon).transform === 'none'), 'opening a certificate must rotate only its chevron');
    for (const width of [760, 1200]) {
      await page.setViewportSize({width,height:800});await page.locator('#usb-connection').scrollIntoViewIfNeeded();
      assert(await page.evaluate(() => document.querySelector('.desktop-content').scrollWidth <= document.querySelector('.desktop-content').clientWidth));
      assert(await page.locator('#usb-connection button').evaluateAll(buttons => buttons.every(button => button.scrollWidth <= button.clientWidth)));
      await page.screenshot({path:resolve(output,`usb-${language}-${width}.png`)});
    }
      await page.evaluate(() => {Object.assign(window.__usb.devices[1],{state:'disconnected',selected:true});window.__usb.activeCount=2;});
      await page.locator('.usb-tools > button').first().click();
      await expect(page.locator('.usb-devices li').nth(1).locator('button')).toBeEnabled();
      await page.locator('.usb-devices li').nth(1).locator('button').click();
      assert(await page.evaluate(() => window.__usb.devices[2].linked && window.__usb.activeCount === 1));
      assert.equal(await page.evaluate(() => window.__usbCalls.at(-1).id),'b'.repeat(32));
      await page.evaluate(() => {Object.assign(window.__usb.devices[0],{state:'ready',selected:true,error:'USB 入口被占用，请断开其它 USB 遥控工具后重试'});window.__usb.devices[1].selected=true;window.__usb.activeCount=3;});
      await page.locator('.usb-tools > button').first().click();
      await expect(page.locator('.usb-devices li').nth(0).locator('.error')).toBeVisible();
      await expect(page.locator('.usb-devices li')).toHaveCount(3);
      assert(await page.evaluate(() => window.__usb.devices[2].linked));
      for (const width of [760,1200]) {
        await page.setViewportSize({width,height:800});await page.locator('#usb-connection').scrollIntoViewIfNeeded();
        assert(await page.evaluate(() => document.querySelector('.desktop-content').scrollWidth <= document.querySelector('.desktop-content').clientWidth));
        await page.screenshot({path:resolve(output,`usb-recovery-${language}-${width}.png`)});
      }
      await page.evaluate(() => {window.__usb.devices[0].error='';window.__usbDetectionFails=true;});
      await page.locator('.usb-tools > button').first().click();
      await expect(page.locator('#usb-connection > .details-body > [role="alert"]')).toBeVisible();
      await page.evaluate(() => window.__usbDetectionFails=false);
      await page.locator('.usb-tools > button').first().click();
      await expect(page.locator('#usb-connection > .details-body > [role="alert"]')).toHaveCount(0);
  }
  assert.deepEqual(await page.evaluate(() => window.__usbCalls.filter(call => call.action === 'connect').map(call => call.id)), Array(3).fill('c'.repeat(32)));
  checks.push('USB device authorization/offline states, explicit selected phone, pairing/certificate display and three-language layouts at two window widths');
  }
  await page.getByRole('button', { name: /^(偏好设置|偏好設定|Preferences)$/ }).click();
  await page.evaluate(() => { window.__state.running = true; window.__state.lanRunning = true; });
  await expect(page.locator('#network-help')).toBeVisible();
  await page.evaluate(() => {
    Object.assign(window.__usb, { available: true, activeCount: 1, endpoint: '127.0.0.1:49123', fingerprint: Array(32).fill('AB').join(':'), code: '246802', error: '' });
    document.querySelector('#usb-connection').open = true;
  });
  await page.locator('.usb-tools > button').first().click();
  await expect(page.locator('.usb-certificate')).toBeVisible();
  for (const language of ['zh-CN', 'zh-TW', 'en']) {
    await page.locator('#desktop-language').selectOption(language);
    for (const message of ['内置 USB 组件缺失，请重新安装电脑端羽连', '无法运行 USB 组件，请重新安装电脑端羽连']) {
      await page.evaluate(message => { window.__usb.error = message; }, message);
      await page.locator('.usb-tools > button').first().click();
      const alert = page.locator('#usb-connection > .details-body > [role="alert"]');
      await expect(alert).toBeVisible();
      if (language === 'en') await expect(alert).toContainText('Reinstall PlumeLink');
      if (language === 'zh-TW') await expect(alert).toContainText('重新安裝');
    }
    await page.evaluate(() => { window.__usb.error = ''; });
    await page.locator('.usb-tools > button').first().click();
    assert(!await page.locator('#usb-connection').innerText().then(text => /adb\.exe|Platform-Tools/.test(text)));
    for (const width of [480, 760, 1200]) {
      await page.setViewportSize({ width, height: 800 });
      await page.evaluate(() => {
        document.querySelectorAll('[data-view=settings] > .settings').forEach(section => section.open = true);
        document.querySelector('.settings-subsection').open = true;
        document.querySelector('.usb-certificate').open = false;
      });
      const geometry = await page.evaluate(() => {
        const rect = selector => document.querySelector(selector).getBoundingClientRect();
        const network = rect('#network'), action = rect('.preference-field > button'), hint = rect('#network-help');
        const subsection = document.querySelector('.settings-subsection');
        const parent = subsection.parentElement.getBoundingClientRect(), child = subsection.getBoundingClientRect();
        const style = getComputedStyle(subsection);
        const certificate = rect('.usb-certificate > summary');
        return {
          networkAligned: Math.abs(network.left - hint.left) < 1 && hint.top >= network.bottom + 4,
          networkActionGrouped: action.left >= network.left - 1 && (action.top >= network.bottom || action.left >= network.right),
          subsectionContained: child.left >= parent.left && child.right <= parent.right,
          subsectionHasNoCard: style.borderLeftWidth === '0px' && style.borderRadius === '0px',
          certificateCompact: certificate.height <= 40,
          overflow: document.querySelector('.desktop-content').scrollWidth > document.querySelector('.desktop-content').clientWidth,
        };
      });
      assert(geometry.networkAligned && geometry.networkActionGrouped, `network field/actions scattered in ${language}/${width}`);
      assert(geometry.subsectionContained && geometry.subsectionHasNoCard, `nested settings hierarchy failed in ${language}/${width}`);
      assert(geometry.certificateCompact && !geometry.overflow, `certificate spacing or settings overflow in ${language}/${width}`);
      for (const [name, selector] of [['connection', '#desktop-settings'], ['usb', '#usb-connection'], ['remote', '.settings-subsection']]) {
        await page.locator(selector).screenshot({ path: resolve(output, `settings-detail-${name}-${language}-${width}.png`) });
      }
    }
  }
  checks.push('three-language settings hierarchy, grouped network controls, compact certificate disclosure and nested shortcut section at 480/760/1200px');
  for (const language of ['zh-CN', 'zh-TW', 'en']) {
    await page.locator('nav > button').nth(3).click();
    await page.locator('#desktop-language').selectOption(language);
    await page.evaluate(() => window.__state.sharedFiles = [
      {id:501,name:'Shared document.txt',size:1024},
      {id:502,name:'Very long document name_'.repeat(20)+'.pdf',size:2048},
      {id:503,name:'Empty file.txt',size:0},
    ]);
    await page.locator('nav > button').nth(2).click();
    await expect(page.locator('.shared-files li')).toHaveCount(3);
    const shared = page.locator('.shared-file-panel');
    for (const width of [480, 760, 1200]) {
      await page.setViewportSize({width,height:800});
      assert(await shared.evaluate(panel => panel.scrollWidth <= panel.clientWidth), `shared list overflows in ${language}/${width}`);
      assert(await page.locator('.file-browse-tools input, .file-browse-tools select').evaluateAll(controls => controls.every(control => {
        const field=control.getBoundingClientRect(), header=control.closest('header').getBoundingClientRect();
        const search=control.closest('header').querySelector('input').getBoundingClientRect();
        return field.left >= header.left + 12 && field.right <= header.right - 12 && field.top >= header.top + 12 && Math.abs(field.top-search.top) <= 1;
      })), `search/sort touch the card edge in ${language}/${width}`);
      await shared.screenshot({path:resolve(output,`shared-list-${language}-${width}.png`)});
    }
    await page.locator('.file-search input').fill('no-matches-fixture');
    await expect(page.locator('.file-empty-state button')).toBeVisible();
    await page.locator('.file-empty-state button').click();
    await expect(page.locator('.shared-files li')).toHaveCount(3);
    await page.evaluate(() => window.__state.sharedFiles = []);
    await expect(page.locator('.file-list-toolbar')).toHaveCount(0);
    await expect(page.locator('.file-browse-tools')).toHaveCount(0);
    await shared.screenshot({path:resolve(output,`shared-empty-${language}.png`)});
  }
  checks.push('shared-file search/sort inset, compact empty state without unavailable controls, three-language long filenames and clear-search recovery at three widths');
  assert.deepEqual(errors, []);
  await writeFile(resolve(output, 'results.json'), JSON.stringify({ scope: 'Production assets with simulated bridge, not Windows native acceptance', checks, errors }, null, 2));
  console.log(`Desktop UI checks passed; screenshots: ${output}`);
} finally {
  await browser?.close();
  await new Promise(resolve => server.close(resolve));
}
