// Uses WebView2's opt-in local debugging only in this developer test process.
import { spawn, execFileSync } from "node:child_process";
import { resolve, join } from "node:path";
import { tmpdir } from "node:os";
import { chromium } from "@playwright/test";
import { writeFile, mkdir, mkdtemp, rm, rmdir } from "node:fs/promises";
import assert from "node:assert/strict";
import { once } from "node:events";
const exe = resolve(
  process.env.PLUMELINK_EXECUTABLE ||
    "src-tauri/target/release/PlumeLink.exe",
);
const port = 19331;
// The acceptance target intentionally serves its pinned, locally generated TLS identity.
process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";
await mkdir("output/playwright", { recursive: true });
await mkdir("artifacts", { recursive: true });
const env = {
  ...process.env,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  Path: `${process.env.SystemRoot}\\System32;${process.env.SystemRoot}`,
};
delete env.PATH;
const app = spawn(exe, [], { env, cwd: process.env.TEMP, windowsHide: true });
const exit = once(app, "exit");
let browser;
const checks = {};
try {
  for (let i = 0; i < 100; i++) {
    if (
      await fetch(`http://127.0.0.1:${port}/json/version`).then(
        (r) => r.ok,
        () => false,
      )
    )
      break;
    await new Promise((r) => setTimeout(r, 200));
  }
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const context = browser.contexts()[0];
  let page;
  for (let i = 0; i < 50; i++) {
    page = context.pages()[0];
    if (page) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  await page.getByRole("button", { name: "停止遥控", exact: true }).waitFor();
  assert.equal(await page.title(), "羽连 PlumeLink");
  checks.desktopStarted = true;
  const invoke = (cmd, args = {}) =>
    page.evaluate(
      ({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args),
      { cmd, args },
    );
  const state = await invoke("desktop_state");
  assert(
    state.running &&
      state.code.length === 6 &&
      state.addresses.some((a) => !a.virtual),
  );
  checks.localCommandsAndAdapters = true;
  const diagnostic = await invoke("diagnostic_action");
  assert(Number.isFinite(diagnostic.pointer.x) && Number.isFinite(diagnostic.pointer.y));
  assert(!("title" in diagnostic) && !("foreground" in diagnostic));
  checks.sanitizedNativeDiagnostics = true;
  await page.getByRole("button", { name: /^设备管理/ }).click();
  const deviceManager = page.locator("#desktop-devices");
  await deviceManager.getByText("多人控制", { exact: true }).waitFor();
  checks.deviceManagementVisible = true;
  const origin = `https://${state.selected}:${state.port}`;
  checks.browserUiBlocked = await fetch(origin).then((r) => r.status === 404);
  checks.qrRendered = state.trusted
    ? true
    : await page
        .locator(".qr-card img")
        .evaluate((img) => img.complete && img.naturalWidth > 0);
  checks.updateUiRemoved =
    (await page.getByText(/检查更新|打开发布页|软件更新/).count()) === 0;
  checks.contextMenuSuppressed = await page.evaluate(() => {
    const probe = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    document.body.dispatchEvent(probe);
    return probe.defaultPrevented;
  });
  await page.getByRole("button", { name: "文件传输", exact: true }).click();
  const transfer = page.locator("#desktop-transfer");
  await transfer.getByText("Windows 接收目录", { exact: true }).waitFor();
  checks.fileTransferVisible = true;
  const fixtureDir = await mkdtemp(join(tmpdir(), "plumelink-desktop-share-"));
  const fixtureNames = ["desktop-batch-a.txt", "desktop-batch-b.txt", "desktop-batch-c.txt"];
  const fixturePaths = fixtureNames.map(name => join(fixtureDir, name));
  let fixtureIds = [];
  try {
    for (const path of fixturePaths) await writeFile(path, "Desktop sharing regression fixture");
    const previousShares = (await invoke("desktop_state")).sharedFiles.map(file => file.id);
    const added = await invoke("transfer_action", { action: "addpaths", paths: fixturePaths });
    fixtureIds = added.sharedFiles.filter(file => !previousShares.includes(file.id)).map(file => file.id);
    await page.getByRole("checkbox", { name: `选择 ${fixtureNames[0]}`, exact: true }).waitFor();
    const listHeight = await page.locator(".shared-file-viewport").evaluate(element => element.getBoundingClientRect().height);
    await page.getByRole("button", { name: `取消共享 ${fixtureNames[0]}`, exact: true }).click();
    await page.getByRole("checkbox", { name: `选择 ${fixtureNames[0]}`, exact: true }).waitFor({ state: "detached" });
    for (const name of fixtureNames.slice(1)) await page.getByRole("checkbox", { name: `选择 ${name}`, exact: true }).check();
    await page.getByRole("button", { name: /^取消所选共享/ }).click();
    await page.getByRole("checkbox", { name: `选择 ${fixtureNames[1]}`, exact: true }).waitFor({ state: "detached" });
    const shares = (await invoke("desktop_state")).sharedFiles;
    assert.deepEqual(shares.map(file => file.id), previousShares);
    assert.equal(await page.locator(".shared-file-viewport").evaluate(element => element.getBoundingClientRect().height), listHeight);
    checks.nativeBatchShareAndCancellation = true;
  } finally {
    const ownShares = (await invoke("desktop_state")).sharedFiles.filter(file => fixtureIds.includes(file.id));
    if (ownShares.length) await invoke("transfer_action", { action: "removemany", ids: ownShares.map(file => file.id) });
    for (const path of fixturePaths) await rm(path, { force: true });
    await rmdir(fixtureDir);
  }

  checks.unknownCommandDenied = await invoke("execute_shell", {
    command: "never-run",
  }).then(
    () => false,
    () => true,
  );
  await page.screenshot({
    path: "output/playwright/windows-file-transfer.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "总览", exact: true }).click();
  await page.getByRole("button", { name: "停止遥控", exact: true }).click();
  await page.getByRole("button", { name: "启动遥控", exact: true }).waitFor();
  checks.stopClosesPort = await fetch(origin).then(
    () => false,
    () => true,
  );
  await page.getByRole("button", { name: "启动遥控", exact: true }).click();
  await page.getByRole("button", { name: "停止遥控", exact: true }).waitFor();
  checks.restart = true;
  const native = (action) =>
    JSON.parse(
      execFileSync(
        "powershell",
        [
          "-NoProfile",
          "-ExecutionPolicy",
          "Bypass",
          "-File",
          resolve("scripts/verify-window.ps1"),
          "-TargetProcessId",
          String(app.pid),
          "-Action",
          action,
        ],
        { encoding: "utf8" },
      ),
    );
  // Tao retains WS_CAPTION for window behavior but removes its visual area in
  // WM_NCCALCSIZE. Verify actual client geometry instead of that style bit.
  const frame = native("state");
  checks.nativeCaptionRemoved = frame.topInset <= Math.ceil(8 * frame.dpi / 96);
  checks.nativeFrameRemoved = frame.clientFillsWindow;
  checks.compositorRenderingEnabled = frame.compositorRenderingEnabled;
  await page.getByRole('button', { name: '最大化', exact: true }).click();
  await page.getByRole('button', { name: '还原窗口', exact: true }).waitFor();
  checks.customMaximize = native("state").maximized;
  await page.getByRole('button', { name: '还原窗口', exact: true }).click();
  await page.getByRole('button', { name: '最大化', exact: true }).waitFor();
  checks.customRestore = !native("state").maximized;
  native("resize");
  const restoredFrame = native("state");
  checks.shadowSurvivesRestore = restoredFrame.clientFillsWindow && restoredFrame.compositorRenderingEnabled;
  checks.internalScrollOnly = await page.evaluate(() => {
    const content = document.querySelector('.desktop-content');
    const titlebar = document.querySelector('.window-titlebar').getBoundingClientRect();
    const rect = content.getBoundingClientRect();
    content.scrollTop = content.scrollHeight;
    return rect.top === titlebar.bottom && rect.bottom === innerHeight && scrollY === 0 && titlebar.width === innerWidth;
  });
  await page.getByRole('button', { name: '关闭窗口', exact: true }).click();
  await new Promise((r) => setTimeout(r, 250));
  checks.closeHidesToTray = !native("state").visible;
  checks.hiddenServiceStillRuns = (await fetch(origin)).status === 404;
  async function reopen() {
    const next = spawn(exe, [], { env, cwd: process.env.TEMP, windowsHide: true });
    await once(next, "exit");
    await new Promise(r => setTimeout(r, 250));
    return native("state");
  }
  const usable = state => state.visible && !state.minimized && state.onMonitor &&
    state.width >= Math.floor(320 * state.dpi / 96) && state.height >= Math.floor(300 * state.dpi / 96);
  checks.singleInstanceRestores = usable(await reopen());
  await page.getByRole('button', { name: '最小化', exact: true }).click();
  await new Promise(r => setTimeout(r, 250));
  const inTray = native("state");
  checks.minimizeHidesToTray = !inTray.visible && !inTray.minimized;
  const fromTray = await reopen();
  checks.trayRestoreKeepsSize = usable(fromTray) &&
    fromTray.width === restoredFrame.width && fromTray.height === restoredFrame.height;

  // The shell uses these same system commands independently of the titlebar.
  native("minimize");
  const minimized = native("state");
  checks.systemMinimizeKeepsTaskbar = minimized.minimized && minimized.visible;
  native("restore");
  checks.taskbarRestore = usable(native("state"));
  native("minimize");
  checks.relaunchFromSystemMinimize = usable(await reopen());
  native("offscreen");
  checks.recoversOffscreenWindow = !native("state").onMonitor && usable(await reopen());
  native("offscreen");
  native("minimize");
  native("restore");
  checks.taskbarRecoversOffscreen = usable(native("state"));
  const tiny = native("tiny");
  checks.recoversTinyWindow = tiny.width <= 120 && tiny.height <= 80 && usable(await reopen());
  await invoke("desktop_action", { action: "quit" }).catch(() => {});
  const code = await Promise.race([
    exit,
    new Promise((_, reject) =>
      setTimeout(() => reject(Error("Quit timeout")), 8000),
    ),
  ]);
  checks.quitClean = code[0] === 0;
  checks.exitClosesPort = await fetch(origin).then(
    () => false,
    () => true,
  );
  await writeFile(
    "artifacts/tauri-desktop-verification.json",
    JSON.stringify({ exe, at: new Date().toISOString(), checks }, null, 2),
  );
  for (const [name, value] of Object.entries(checks)) assert(value, name);
  console.log(checks);
} finally {
  if (browser) await browser.close().catch(() => {});
  if (app.exitCode === null) app.kill();
}
