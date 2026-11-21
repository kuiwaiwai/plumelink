// Production desktop assets with a simulated Tauri bridge; no native actions.
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import assert from 'node:assert/strict';
import { chromium } from '@playwright/test';
import QRCode from 'qrcode';

const root = resolve('tauri-dist/desktop-ui');
const output = resolve('artifacts/desktop-ui');
await mkdir(output, { recursive: true });
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
    window.__haloRequests = [];
    window.__pickedFiles = [];
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
          if (args.action === 'setup') {
            if (!['cable', 'plume'].includes(args.id)) throw Error('No verified cable');
            window.__audioSelection = args.id;
            return {selected: args.id, target: {name: args.id === 'plume' ? 'Microphone (PlumeLink Microphone)' : 'CABLE Output (test)'}};
          }
          if (args.action === 'installCable') {
            if (window.__cancelInstaller) throw Error('已取消管理员授权。');
            window.__installedCable = true;
          }
          if (args.action === 'select') window.__audioSelection = args.id;
          if (args.action === 'stop') window.__state.microphone.active = false;
          return { cableInstalled: window.__serviceOnly || window.__installedCable, outputs: [{ id: 'plume', name: 'Speakers (PlumeLink Microphone)', virtualHint: true, plumeLink: true }, { id: 'cable', name: 'CABLE Input (test)', virtualHint: true }, { id: 'uu', name: '扬声器 (UU远程虚拟音频设备)', virtualHint: true }].filter(o => o.id !== 'cable' || window.__installedCable), recordingInputs: structuredClone(window.__recordingInputs), selected: window.__audioSelection };
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
  await page.setViewportSize({ width: 1200, height: 650 });
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
  await page.evaluate(() => { window.__serviceOnly = false; window.__cancelInstaller = true; });
  await page.getByRole('button', { name: '刷新设备', exact: true }).click();
  await page.getByRole('button', { name: '安装 VB-CABLE' }).click();
  await page.getByText('已取消管理员授权。', { exact: true }).waitFor();
  await page.evaluate(() => window.__cancelInstaller = false);
  await page.getByRole('button', { name: '安装 VB-CABLE' }).click();
  await page.getByText('录音端已收到测试音，请在收音软件中选择「CABLE Output (test)」。').waitFor();
  assert.equal(await page.getByRole('button', { name: '安装 VB-CABLE' }).count(), 0);
  await page.getByRole('button', { name: '检测并配置麦克风' }).click();
  await page.waitForFunction(() => window.__audioSelection === 'cable');
  await page.getByRole('combobox', { name: '麦克风输出设备' }).selectOption('cable');
  await page.getByRole('button', { name: '播放测试音并配置' }).click();
  await page.getByText('录音端已收到测试音，请在收音软件中选择「CABLE Output (test)」。').waitFor();
  await page.getByRole('button', { name: '保存音频设备', exact: true }).click();
  await page.waitForFunction(() => window.__audioSelection === 'cable');
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
  await page.screenshot({ path: resolve(output, 'microphone.png'), fullPage: true });
  const longDeviceName = 'LivingRoom-Workstation-'.repeat(12);
  await page.evaluate(name => window.__state.microphone.deviceName = name, longDeviceName);
  await page.getByText(`已收到音频 · ${longDeviceName}`, {exact:true}).waitFor();
  await page.setViewportSize({width:480,height:760});
  await page.locator('.microphone-settings summary').scrollIntoViewIfNeeded();
  assert(await page.locator('.microphone-settings summary').evaluate(e=>e.scrollWidth<=e.clientWidth));
  await page.screenshot({path:resolve(output,'microphone-long-name-480.png')});
  await page.evaluate(()=>window.__state.microphone.deviceName='测试手机');
  checks.push('long active microphone device names wrap within the summary at minimum desktop width');
  await page.getByRole('button', { name: '连接帮助', exact: true }).click();
  await page.getByRole('button', { name: '打开诊断', exact: true }).click();
  await page.waitForSelector('#desktop-diagnostics[open] summary:focus');
  await page.getByText('麦克风阵列 (UU远程虚拟音频设备)', { exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: '检测通路', exact: true }).count(), 1);
  await page.evaluate(() => window.__recordingInputs = []);
  await page.getByRole('button', { name: '刷新录音设备', exact: true }).click();
  await page.getByText('未发现可用录音设备。请安装并启用虚拟音频线缆，再刷新设备。', { exact: true }).waitFor();
  await page.screenshot({ path: resolve(output, 'microphone-diagnostics.png'), fullPage: true });
  await page.setViewportSize({ width: 480, height: 760 });
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  checks.push('audio opening, silent packets, signal reception and submitted output are distinguished', 'recording devices listed without inferring loopback from virtual names');
  await page.getByRole('button', { name: '偏好设置', exact: true }).click();
  await page.getByRole('button', { name: '停止接收', exact: true }).click();
  await page.waitForFunction(() => !window.__state.microphone.active);
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
  assert.equal(await fileViewport.evaluate(element => element.getBoundingClientRect().height), emptyHeight);
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
  const toolbarFull = await page.locator('.file-list-toolbar').boundingBox();
  await page.getByRole('button', { name: '全部取消共享', exact: true }).click();
  await page.getByText('暂无共享文件，选择后会显示在这里。', { exact: true }).waitFor();
  assert.equal(await fileViewport.evaluate(element => element.getBoundingClientRect().height), emptyHeight);
  assert.equal((await page.locator('.file-list-toolbar').boundingBox()).y, toolbarFull.y);
  assert(await page.getByRole('button', { name: '全部取消共享', exact: true }).isDisabled());
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
  assert.deepEqual(errors, []);
  await writeFile(resolve(output, 'results.json'), JSON.stringify({ scope: 'Production assets with simulated bridge, not Windows native acceptance', checks, errors }, null, 2));
  console.log(`Desktop UI checks passed; screenshots: ${output}`);
} finally {
  await browser?.close();
  await new Promise(resolve => server.close(resolve));
}
