import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { expect } from '@playwright/test';

export async function verifyDesktopReceive(page, output) {
  const records = [
    { taskId: 'active', name: '设计资料_屏幕与文件布局_'.repeat(12) + '.zip', size: 1048576, receivedBytes: 524288, state: 'receiving', deviceName: '测试手机' },
    { taskId: 'saving', name: 'notes.txt', size: 1024, receivedBytes: 1024, state: 'saving' },
    { taskId: 'retry', name: 'unfinished.pdf', size: 4096, receivedBytes: 256, state: 'stopping', error: '无法清理未完成文件，请重试停止' },
    { taskId: 'saved', name: 'photo (1).jpg', size: 2048, receivedBytes: 2048, state: 'completed', deviceName: 'Phone' },
    { taskId: 'stopped', name: 'stopped.txt', size: 10, receivedBytes: 4, state: 'stopped' },
  ];
  const panel = page.locator('.received-panel');
  const row = id => panel.locator(`[data-task="${id}"]`);
  const dicts = {
    'zh-CN': {},
    'zh-TW': JSON.parse(await readFile('shared/locales/zh-Hant.json', 'utf8')),
    en: JSON.parse(await readFile('shared/locales/en.json', 'utf8')),
  };
  const checks = [];
  for (const [language, dict] of Object.entries(dicts)) {
    const text = key => dict[key] || key;
    await page.locator('nav > button').nth(3).click();
    await page.locator('#desktop-language').selectOption(language);
    await page.evaluate(files => {
      window.__state.receivedFiles = files;
      window.__state.receivePath = 'Downloads\\' + '长接收目录-'.repeat(30);
    }, records);
    await page.locator('nav > button').nth(2).click();
    await expect(row('active')).toBeVisible();
    assert.equal(await panel.locator('summary > svg').first().evaluate(element => getComputedStyle(element).transform), 'none', 'disclosure rotation must only affect the chevron');
    await expect(panel.locator('.receive-count')).toHaveText('3');
    await expect(row('active').locator('progress')).toHaveAttribute('value', '50');
    await expect(row('saving').locator('.receive-status')).toHaveText(text('正在保存'));
    await expect(row('saving').locator('progress')).toHaveAttribute('value', '100');
    await expect(row('retry').locator('.receive-file-error')).toHaveText(text('无法清理未完成文件，请重试停止'));
    await expect(row('saved').locator('.receive-status')).toHaveText(text('已保存'));
    for (const width of [760, 1200]) {
      await page.setViewportSize({ width, height: 850 });
      await page.emulateMedia({ colorScheme: width === 760 ? 'light' : 'dark' });
      await panel.scrollIntoViewIfNeeded();
      assert(await panel.evaluate(element => element.scrollWidth <= element.clientWidth), 'receive panel must not overflow');
      assert(await panel.locator('.transfer-setting small').evaluate(element => element.clientHeight <= parseFloat(getComputedStyle(element).lineHeight) * 2 + 1), 'long receiving paths are limited to two lines with a full-path title');
      assert(await row('active').locator('.receive-name').evaluate(element => element.clientHeight <= parseFloat(getComputedStyle(element).lineHeight) * 2 + 1), 'long names are limited to two lines');
      assert(await row('saved').locator('button').evaluate(element => element.clientWidth >= 34 && element.clientHeight >= 34));
      await page.screenshot({ path: resolve(output, `receive-${language}-${width}.png`) });
    }
    checks.push(`${language}: receiving/saving/stopping/saved states, long names and directories at 760/1200 widths, light/dark`);
  }

  await panel.locator('summary').click();
  await expect(panel.locator('details')).not.toHaveAttribute('open', '');
  await page.waitForTimeout(1100);
  await expect(panel.locator('details')).not.toHaveAttribute('open', '');
  await panel.locator('summary').click();
  await page.evaluate(() => { window.__holdReceiveOpen = true; });
  await row('saved').locator('button').click();
  await page.waitForFunction(() => !!window.__finishReceiveOpen);
  await expect(row('active').locator('button')).toBeEnabled();
  await row('active').locator('button').click();
  await expect(row('active')).toHaveAttribute('data-state', 'stopping');
  await expect(row('active').locator('button')).toBeDisabled();
  await page.evaluate(() => {
    window.__state.receivedFiles.unshift({ taskId: 'new', name: 'new.txt', size: 0, receivedBytes: 0, state: 'receiving' });
  });
  await expect(row('new')).toBeVisible();
  await page.evaluate(() => window.__finishReceiveOpen());
  await expect(row('saved').locator('button')).toBeEnabled();
  await expect(row('new')).toBeVisible();
  await expect(row('active')).toHaveAttribute('data-state', 'stopping');
  assert.deepEqual(await page.evaluate(() => window.__receiveCalls.filter(call => call.action === 'open').map(call => call.taskId)), ['saved']);
  checks.push('slow folder-open reply cannot overwrite newer tasks; progress and cancellation stay usable, correct original-folder task ID');

  await page.evaluate(() => { window.__failReceiveCancel = true; });
  await row('retry').locator('button').click();
  await expect(panel.locator('[role="alert"]')).toBeVisible();
  await expect(row('retry').locator('button')).toBeEnabled();
  await page.evaluate(() => { window.__failReceiveCancel = false; });
  await row('retry').locator('button').click();
  await expect(row('retry').locator('button')).toBeDisabled();
  await expect(panel.locator('[role="alert"]')).toHaveCount(0);
  await panel.locator('.receive-toolbar button').click();
  await expect(row('saved')).toHaveCount(0);
  await expect(row('stopped')).toHaveCount(0);
  await expect(row('new')).toBeVisible();
  await expect(row('saving')).toBeVisible();
  await page.evaluate(() => { window.__holdReceiveOpen = false; window.__failReceiveOpen = true; });
  await panel.locator('.receive-folder-actions button').first().click();
  await expect(panel.locator('[role="alert"]')).toBeVisible();
  await expect(panel.locator('.receive-folder-actions button').first()).toBeEnabled();
  assert.equal(await page.evaluate(() => window.__receiveCalls.filter(call => call.action === 'clear').length), 1);
  checks.push('cancel failure recovers to retry; clear keeps unfinished tasks; missing directory error releases local busy state; collapse survives polling');
  return checks;
}
