import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { expect } from '@playwright/test';

export async function verifyConnectionRecovery({ page, context, fire, control, commands, errors, output }) {
  const checks = [];
  let revision = 100;
  const calls = () => page.evaluate(() => window.__nativeCalls.filter(call => ['recover', 'cancel-recovery'].includes(call[0])));
  for (const language of ['zh-CN', 'zh-TW', 'en']) {
    const dictionary = language === 'zh-CN' ? {} : JSON.parse(await readFile(resolve('shared/locales', language === 'en' ? 'en.json' : 'zh-Hant.json'), 'utf8'));
    const tr = key => dictionary[key] || key;
    await fire('nav-settings'); await fire('language'); await fire('language' + language);
    await fire('nav-remote'); await fire('keyboard'); await control('draft');
    const draft = `Draft ${language}\n第二行 😀`;
    await fire('draft', { text: draft, revision: revision++ });
    await page.evaluate(() => window.plumeLinkBack());
    await context.setOffline(true); await page.evaluate(() => window.__nativeControlSocket.close());
    await page.waitForFunction(() => window.__nativeUi.sessionStatus !== 'connected');
    await fire('connection-chip'); await control('connection-find', { label: tr('查找并重新连接'), enabled: true });
    const before = (await calls()).filter(call => call[0] === 'recover').length;
    await fire('connection-find'); await fire('connection-find');
    await control('connection-find', { label: tr('正在查找这台电脑…'), enabled: false });
    const started = (await calls()).filter(call => call[0] === 'recover');
    assert.equal(started.length, before + 1); const request = started.at(-1)[2];
    assert.match(request, /^[a-f0-9]{32}$/); assert.equal(started.at(-1)[1], draft);
    await page.evaluate(({ request, message }) => window.dispatchEvent(new CustomEvent('plumelink-connection-recovery', {
      detail: { request, phase: 'error', message },
    })), { request, message: '请先停止收音并等待文件传输结束，再恢复连接。' });
    await control('connection-find-error', { label: tr('请先停止收音并等待文件传输结束，再恢复连接。') });
    await control('connection-find', { enabled: true });
    await fire('connection-find'); const second = (await calls()).filter(call => call[0] === 'recover').at(-1)[2];
    await fire('dismiss');
    await expect.poll(async () => (await calls()).filter(call => call[0] === 'cancel-recovery').at(-1)?.[1]).toBe(second);
    await context.setOffline(false); await page.evaluate(() => window.dispatchEvent(new Event('online')));
    await page.waitForFunction(() => window.__nativeUi.sessionStatus === 'connected');
    await fire('keyboard'); await control('draft', { value: draft }); await page.evaluate(() => window.plumeLinkBack());
    checks.push({ language, explicitOnly: true, doubleActivationOnce: true, busyErrorTranslated: true, cancelOnDismiss: true, draftKept: true });
  }
  await page.evaluate(() => sessionStorage.setItem('recovery-draft', 'Recovered draft\n保留 😀'));
  await page.reload(); await page.waitForFunction(() => window.__nativeUi?.sessionStatus === 'connected');
  await fire('keyboard'); await control('draft', { value: 'Recovered draft\n保留 😀' });
  assert.equal(await page.evaluate(() => sessionStorage.getItem('recovery-draft')), null);
  assert.equal(commands.filter(command => command.type !== 'release').length, 0, 'recovery and draft restoration must never emit input or launch commands');
  assert.deepEqual(errors, []);
  await writeFile(resolve(output, 'connection-recovery-results.json'), JSON.stringify({ passed: true, checks,
    scope: 'Production mobile native UI schema, real WASM/WSS/Fake Native; recovery discovery and native lifecycle are simulated here and covered separately in Java tests.',
    draftRestoredWithoutSending: true, errors }, null, 2), { flag: 'wx' });
  console.log('Connection recovery UI passed in three languages without replaying input.');
}
