import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { expect } from '@playwright/test';

export async function verifyDesktopConnections(page, output) {
  await page.setViewportSize({ width: 1200, height: 850 });
  const translations = {
    'zh-CN': {},
    'zh-TW': JSON.parse(await readFile('shared/locales/zh-Hant.json', 'utf8')),
    en: JSON.parse(await readFile('shared/locales/en.json', 'utf8')),
  };
  const checks = [];
  for (const language of Object.keys(translations)) {
    const text = key => translations[language][key] || key;
    await page.locator('nav > button').nth(3).click();
    await page.locator('#desktop-language').selectOption(language);
    for (const state of [
      { name: 'wireless', running: true, lanRunning: true, active: true, error: '', discovery: '已启动广播' },
      { name: 'wireless-no-broadcast', running: true, lanRunning: true, active: false, error: '', discovery: '未广播' },
      { name: 'wireless-error', running: true, lanRunning: true, active: false, error: 'fixture discovery failure', discovery: '广播启动失败' },
      { name: 'usb', running: true, lanRunning: false, active: false, error: 'stale discovery failure', discovery: '未运行' },
      { name: 'usb-saved', running: true, lanRunning: false, active: false, error: '', trusted: true, discovery: '未运行' },
      { name: 'stopped', running: false, lanRunning: false, active: true, error: 'stale discovery failure', discovery: '未运行' },
    ]) {
      await page.evaluate(next => Object.assign(window.__state, {
        running: next.running, lanRunning: next.lanRunning, connected: false, connectedCount: 0,
        trusted: !!next.trusted, pairedCount: next.trusted ? 1 : 0,
        controlMode: 'free', discovery: { active: next.active, error: next.error },
        code: '654321', codeRemaining: 600, error: '',
      }), state);
      const usbOnly = state.running && !state.lanRunning;
      await page.locator('nav > button').first().click();
      const overview = page.locator('[data-view="overview"]');
      await expect(overview.locator('.overview > div').nth(2).locator('b')).toHaveText(text(!state.running ? '发现已暂停' : usbOnly ? 'USB 有线连接' : state.active ? '可自动发现' : '使用扫码连接'));
      await expect(overview.locator('.qr-card')).toHaveCount(state.lanRunning ? 1 : 0);
      if (usbOnly) {
        await expect(overview.locator('.connect-guide')).toContainText(text('连接 USB 数据线'));
        await expect(overview.locator('.connect-guide')).not.toContainText(text('扫描电脑二维码'));
        if (state.trusted) {
          await expect(overview.locator('.pair-toggle')).toHaveText(text('配对信息'));
          if (await overview.locator('.pair-toggle').getAttribute('aria-expanded') === 'false') await overview.locator('.pair-toggle').click();
        }
        await expect(overview.locator('.pair-code')).toHaveText('654321');
        assert(await overview.locator('.connection-grid').evaluate(element => element.firstElementChild.getBoundingClientRect().width > element.clientWidth * 0.8), 'USB pairing details must use the space left by the absent QR code');
      }
      if (state.name === 'usb') {
        await page.screenshot({ path: resolve(output, `usb-overview-${language}.png`) });
        await overview.locator('.workspace-heading button').click();
        await expect(page.locator('#usb-connection')).toHaveAttribute('open', '');
        await expect(page.locator('[data-view="settings"]')).toBeVisible();
      }
      await page.locator('.sidebar-bottom').getByRole('button', { name: text('连接帮助'), exact: true }).click();
      const diagnostics = page.locator('#desktop-diagnostics');
      await diagnostics.evaluate(element => { element.open = true; });
      const entry = label => diagnostics.locator('dl > div').filter({ has: page.locator('dt', { hasText: new RegExp('^' + label + '$') }) }).locator('dd');
      await expect(entry(text('自动发现'))).toHaveText(text(state.discovery));
      await expect(entry(text('无线连接'))).toHaveText(text(state.lanRunning ? '运行中' : '未启用'));
      await expect(entry(text('连接方式'))).toHaveCount(usbOnly ? 1 : 0);
      if (usbOnly) await expect(entry(text('连接方式'))).toHaveText(text('USB 有线连接'));
      await diagnostics.getByRole('button', { name: text('复制脱敏诊断信息'), exact: true }).click();
      const report = await page.evaluate(() => window.__clipboard);
      assert(report.includes(text('自由模式')), 'Copied control mode must be localized');
      assert(report.includes(text(state.discovery)), 'Copied discovery status must match the displayed status');
      assert(!report.includes('654321'), 'Copied diagnostics must exclude the pairing code');
      assert.equal(report.includes('fixture discovery failure'), state.name === 'wireless-error');
      assert(!report.includes('stale discovery failure'));
      if (usbOnly) assert(report.includes(text('USB 有线连接')));
      assert(await page.evaluate(() => document.querySelector('.desktop-content').scrollWidth <= document.querySelector('.desktop-content').clientWidth));
      if (state.name === 'usb') {
        await page.screenshot({ path: resolve(output, `usb-diagnostics-${language}.png`) });
        await page.setViewportSize({ width: 480, height: 780 });
        await page.emulateMedia({ colorScheme: 'dark' });
        assert(await page.evaluate(() => document.querySelector('.desktop-content').scrollWidth <= document.querySelector('.desktop-content').clientWidth));
        await page.screenshot({ path: resolve(output, `usb-diagnostics-compact-dark-${language}.png`) });
        await page.locator('nav > button').first().click();
        await expect(overview.locator('.workspace-heading button')).toBeVisible();
        await overview.locator('.workspace-heading button').click();
        await expect(page.locator('[data-view="settings"]')).toBeVisible();
        await expect(page.locator('#usb-connection')).toHaveAttribute('open', '');
        await page.setViewportSize({ width: 1200, height: 850 });
        await page.emulateMedia({ colorScheme: 'light' });
      }
      checks.push(`${language}/${state.name}`);
    }
  }
  return checks;
}
