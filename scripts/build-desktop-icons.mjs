import { readFile, writeFile } from 'node:fs/promises';
import { chromium } from '@playwright/test';

const svg = await readFile('apps/desktop-ui/assets/app-icon.svg', 'utf8');
const browser = await chromium.launch({ channel: process.env.PLAYWRIGHT_CHANNEL || 'chrome', headless: true });
try {
  const page = await browser.newPage({ deviceScaleFactor: 1 });
  await page.setContent(`<style>html,body{margin:0;width:100%;height:100%;background:transparent}svg{display:block;width:100%;height:100%}</style>${svg}`);
  const sizes = [16, 24, 32, 48, 64, 128, 256];
  const frames = [];
  for (const size of sizes) {
    await page.setViewportSize({ width: size, height: size });
    frames.push(await page.screenshot({ omitBackground: true }));
  }
  const header = Buffer.alloc(6 + 16 * sizes.length);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  for (const [index, size] of sizes.entries()) {
    const entry = 6 + index * 16;
    header[entry] = header[entry + 1] = size === 256 ? 0 : size;
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(frames[index].length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += frames[index].length;
  }
  const ico = Buffer.concat([header, ...frames]);
  for (const name of ['icon', 'installer']) {
    await writeFile(`src-tauri/icons/${name}.ico`, ico);
    await writeFile(`src-tauri/icons/${name}.png`, frames.at(-1));
  }
  for (const [size, paths] of [
    [64, ['shared/assets/favicon.png']],
    [180, ['shared/assets/logo-ui.png']],
    [256, ['docs/assets/logo.png']],
    [512, ['shared/assets/logo.png', 'android-app/app/src/main/res/drawable-nodpi/ic_launcher.png']],
  ]) {
    await page.setViewportSize({ width: size, height: size });
    const png = await page.screenshot({ omitBackground: true });
    for (const path of paths) await writeFile(path, png);
  }
  console.log('Generated Windows icons (16–256px), Android launcher and web/project branding from the supplied SVG.');
} finally {
  await browser.close();
}
