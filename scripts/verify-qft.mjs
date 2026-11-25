// Production mobile assets + real WASM/WSS/server/worker; only native effects are simulated.
import { spawn, spawnSync } from "node:child_process";
import { createInterface } from "node:readline";
import { readFile, mkdir, writeFile } from "node:fs/promises";
import { resolve, extname, sep, dirname } from "node:path";
import assert from "node:assert/strict";
import { isDeepStrictEqual } from "node:util";
import { chromium } from "playwright";
import { expect } from "@playwright/test";

const build = spawnSync(
  process.execPath,
  ["scripts/cargo.mjs", "build", "--locked", "--example", "qft_fixture"],
  { stdio: "inherit", windowsHide: true },
);
if (build.status !== 0) throw new Error("QFT fixture build failed");
const outputArgument = process.argv.find(argument => argument.startsWith("--output="));
const requestedOutput = outputArgument?.slice("--output=".length);
if (outputArgument && !requestedOutput) throw new Error("--output requires a directory");
const root = resolve("tauri-dist/mobile"),
  output = resolve(requestedOutput || (process.argv.includes("--usb") ? "artifacts/qft-integration-usb" : "artifacts/qft-integration"));
if (requestedOutput) await mkdir(dirname(output), { recursive: true });
await mkdir(output, { recursive: !requestedOutput });
const mixed = process.argv.includes("--mixed");
const usb = process.argv.includes("--usb");
const launchErrors = process.argv.includes("--launch-errors");
const windowLive = process.argv.includes("--window-live");
const microphoneStatus = process.argv.includes("--microphone-status");
const shortcutPlans = process.argv.includes("--shortcut-plans");
const connectionRecovery = process.argv.includes("--connection-recovery");
const nativeUi = process.argv.includes("--native-ui") || launchErrors || windowLive || microphoneStatus || shortcutPlans || connectionRecovery;
if (launchErrors) await writeFile(resolve(output, "invalid-app.exe"), "MZ");
let leaseMs = 0;
const fixture = spawn(
  resolve(
    `src-tauri/target/debug/examples/qft_fixture${process.platform === "win32" ? ".exe" : ""}`,
  ),
  [...(launchErrors ? ["--launch=" + resolve(output, "invalid-app.exe")] : mixed ? ["--mixed"] : []), ...(usb ? ["--usb"] : []), ...(connectionRecovery ? ["--secondary-loopback"] : [])],
  { stdio: ["pipe", "pipe", "inherit"], windowsHide: true },
);
const lines = createInterface({ input: fixture.stdout });
const commands = [],
  errors = [],
  frames = { sockets: 0, sent: {}, received: {}, stages: 0, lifecycle: [] };
const audioReports = [];
const trace = [];
let browser, page;
try {
  const ready = await new Promise((resolveReady, reject) => {
    const timeout = setTimeout(
      () => reject(new Error("Fixture startup timed out")),
      15000,
    );
    fixture.once("error", reject);
    fixture.once("exit", (code) => reject(new Error(`Fixture exited ${code}`)));
    lines.on("line", (line) => {
      const value = JSON.parse(line);
      if (value.event === "ready") {
        clearTimeout(timeout);
        resolveReady(value);
      }
      if (value.event === "command") commands.push(value.command);
      if (value.event === "audio") audioReports.push(value);
    });
  });
  if (usb) {
    assert.ok(ready.usbOrigin, "USB listener was not started");
    ready.origin = ready.usbOrigin;
  }
  if (usb) assert.match(ready.usbAccess, /^[a-f0-9]{64}$/, "USB fixture access credential is missing");
  const fixtureUsbHeaders = usb ? { "X-PlumeLink-USB": ready.usbAccess } : {};
  browser = await chromium.launch({ channel: "chrome", headless: true });
  // The fixture's ephemeral loopback certificate is the only ignored certificate.
  const context = await browser.newContext({ locale: "zh-CN",
    ignoreHTTPSErrors: true,
    viewport: { width: 390, height: 750 },
    isMobile: true,
    hasTouch: true,
    deviceScaleFactor: 2,
  });
  await context.grantPermissions(["local-network-access"], {
    origin: ready.origin,
  });
  const pairing = await context.request.post(`${ready.origin}/api/pair`, {
    headers: { origin: ready.origin, ...fixtureUsbHeaders },
    maxRedirects: 0,
    data: {
      code: ready.code,
      deviceId: "qft-browser-fixture",
      deviceName: "QFT 验证设备",
    },
  });
  assert.equal(pairing.status(), 200);
  const credentials = await pairing.json();
  await context.addInitScript(({ token, deviceId }) => {
    localStorage.setItem("plumelink-token", token);
    localStorage.setItem("plumelink-device-id", deviceId);
  }, credentials);
  await context.addInitScript(({ useNativeUi, usbAccess, recovery }) => {
      window.__nativeCalls = [];
      window.__folderSelected = true;
      window.PlumeLinkAndroid = {
        scanPairingCode: () => window.__nativeCalls.push(['scan']),
        takePairingCode: () => { const code = sessionStorage.getItem('scanned-code') || ''; sessionStorage.removeItem('scanned-code'); return code; },
        getComputerName: () => "书房电脑",
        getUsbAccessKey: () => usbAccess,
        getDeviceName: () => "验收手机",
        getMobilePreferences: () => localStorage.getItem("test-native-mobile") || "",
        setMobilePreferences: value => localStorage.setItem("test-native-mobile", value),
        getReceiveFolder: () => window.__folderSelected ? "下载 / PlumeLink" : "请选择接收目录",
        hasReceiveFolder: () => window.__folderSelected,
        setAppearance: (dark) =>
          window.__nativeCalls.push(["appearance", dark]),
        setKeepAwake: (enabled) =>
          window.__nativeCalls.push(["awake", enabled]),
        setConnectionKeepAlive: (enabled) =>
          window.__nativeCalls.push(["keep-alive", enabled]),
        setImmersive: (enabled) =>
          window.__nativeCalls.push(["immersive", enabled]),
        startMicrophone: (id) => {
          window.__nativeCalls.push(["mic-start", id]);
          window.__activeMic = id;
          setTimeout(() => {
            if (window.__activeMic === id && !window.__holdMicStart)
              window.dispatchEvent(
                new CustomEvent("plumelink-microphone", {
                  detail: { id, phase: "live", level: 0.58, message: "" },
                }),
              );
          }, 30);
        },
        stopMicrophone: (id) => {
          window.__nativeCalls.push(["mic-stop", id]);
          window.__activeMic = null;
          window.__lockedMic = false;
        },
        lockMicrophone: (id) => {
          if (window.__activeMic === id) window.__lockedMic = true;
        },
        syncSession: () => {
          if (window.__activeMic && window.__lockedMic)
            window.dispatchEvent(new CustomEvent("plumelink-microphone", {
              detail: { id: window.__activeMic, phase: "live", level: .58, locked: true, snapshot: true },
            }));
        },
        haptic: () => window.__nativeCalls.push(["haptic"]),
        chooseReceiveFolder: () => {
          window.__folderSelected = true;
          window.dispatchEvent(
            new CustomEvent("plumelink-native-transfer", {
              detail: { folder: "文档 / 羽连" },
            }),
          );
        },
        downloadFile: (...args) => window.__nativeCalls.push(["download", ...args]),
        chooseUploadFiles: () => window.__nativeCalls.push(["choose-upload"]),
        cancelFileTransfer: () => window.__nativeCalls.push(["cancel-transfer"]),
        resolveUnfinishedDownload: (...args) => window.__nativeCalls.push(["resolve-download", ...args]),
        changeComputer: () => window.__nativeCalls.push(["computers"]),
      };
      if (recovery) Object.assign(window.PlumeLinkAndroid, {
        recoverConnection: (draft, request) => window.__nativeCalls.push(['recover', draft, request]),
        cancelConnectionRecovery: request => window.__nativeCalls.push(['cancel-recovery', request]),
        takeRecoveredDraft: () => { const draft = sessionStorage.getItem('recovery-draft') || ''; sessionStorage.removeItem('recovery-draft'); return draft; },
      });
      if (useNativeUi) {
        window.PlumeLinkAndroid.renderNativeUi = json => { window.__nativeUi = JSON.parse(json); };
        const RealWebSocket = window.WebSocket;
        window.WebSocket = class extends RealWebSocket {
          constructor(...args) {
            super(...args);
            if (new URL(args[0]).pathname === "/ws") window.__nativeControlSocket = this;
          }
        };
      }
    }, { useNativeUi: true, usbAccess: usb ? ready.usbAccess : "", recovery: connectionRecovery });
  if (mixed) {
    const { installMixedAudio, installMixedUpload } = await import("./verify-concurrency.mjs");
    await context.addInitScript(installMixedAudio, credentials);
    await context.addInitScript(installMixedUpload, credentials);
  }
  page = await context.newPage();
  // Mirrors Android's local asset interception; /api and /ws use the real server.
  const serveAssets = async (route) => {
    const url = new URL(route.request().url());
    if (
      url.pathname.startsWith("/api/") ||
      url.pathname === "/ws" ||
      url.pathname === "/microphone"
    ) {
      await route.continue();
      return;
    }
    const path = resolve(
      root,
      "." + (url.pathname === "/" ? "/index.html" : url.pathname),
    );
    if (!path.startsWith(root + sep)) {
      await route.fulfill({ status: 403 });
      return;
    }
    try {
      await route.fulfill({
        status: 200,
        body: await readFile(path),
        headers: {
          "content-type":
            {
              ".html": "text/html; charset=utf-8",
              ".js": "text/javascript",
              ".css": "text/css",
              ".wasm": "application/wasm",
              ".png": "image/png",
            }[extname(path)] || "application/octet-stream",
          "content-security-policy":
            "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'self'",
        },
      });
    } catch {
      await route.fulfill({ status: 404 });
    }
  };
  await page.route(`${ready.origin}/**`, serveAssets);

  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  async function expectHttpFailure(path, status, trigger) {
    const logged = page.waitForEvent("console", message =>
      message.type() === "error" && message.location().url === ready.origin + path &&
      message.text().includes("status of " + status + " ("));
    await trigger();
    const index = errors.indexOf((await logged).text());
    assert.ok(index >= 0);
    errors.splice(index, 1);
  }
  page.on("websocket", (ws) => {
    frames.sockets++;
    const socket = frames.sockets;
    frames.lifecycle.push({ socket, event: "open", at: Date.now() });
    ws.on("close", () =>
      frames.lifecycle.push({ socket, event: "close", at: Date.now() }),
    );
    for (const [event, field] of [
      ["framesent", "sent"],
      ["framereceived", "received"],
    ]) {
      ws.on(event, ({ payload }) => {
        if (
          Buffer.isBuffer(payload) &&
          payload.length === 64 &&
          payload.toString("ascii", 0, 4) === "QFT2"
        ) {
          const kind = payload[4];
          frames[field][kind] = (frames[field][kind] || 0) + 1;
          trace.push({
            at: Date.now(),
            socket,
            event,
            kind,
            lease: payload.readUInt32LE(16),
            version: payload.readUInt32LE(20),
            serial: payload.readUInt32LE(48),
            status: payload.readUInt32LE(52),
          });
          if (trace.length > 128) trace.shift();
        } else if (typeof payload === "string") {
          const message = JSON.parse(payload);
          if (event === "framesent" && message.kind === "qftStage")
            frames.stages++;
          if (event === "framereceived" && message.kind === "ready" && message.qft)
            leaseMs = message.qft.leaseMs;
        }
      });
    }
  });
  const unsupportedContext = await browser.newContext({ ignoreHTTPSErrors: true });
  try {
    await unsupportedContext.addInitScript(({ token, deviceId }) => {
      localStorage.setItem("plumelink-token", token);
      localStorage.setItem("plumelink-device-id", deviceId);
    }, credentials);
    const unsupported = await unsupportedContext.newPage();
    let unexpectedSockets = 0;
    unsupported.on("websocket", () => unexpectedSockets++);
    await unsupported.route(`${ready.origin}/**`, serveAssets);
    await unsupported.goto(ready.origin);
    await unsupported.waitForLoadState("networkidle");
    assert.equal(await unsupported.locator("button,input,nav,dialog").count(), 0);
    assert.equal(await unsupported.locator("body").innerText(), "");
    assert.equal(unexpectedSockets, 0, "the Android runtime must not start without its native bridge");
  } finally { await unsupportedContext.close(); }
  await page.goto(ready.origin);
  const fire = (id, value = null) => page.evaluate(({ id, value }) => {
    const find = items => {
      for (const item of items || []) {
        if (item.id === id) return item;
        const child = find(item.children); if (child) return child;
      }
    };
    const state = window.__nativeUi;
    const control = find([state.connection, ...state.controls, ...state.nav, ...state.sheet, ...state.sheetActions, ...(state.sheetFooter || [])]);
    const actionId = id === 'dismiss' ? state.dismissAction || id : control?.actionId || state.screenActions?.[id] || id;
    window.dispatchEvent(new CustomEvent("plumelink-native-ui", { detail: { id: actionId, value } }));
  }, { id, value });
  const control = (id, expected = {}) => page.waitForFunction(({ id, expected }) => {
    const find = items => {
      for (const item of items || []) {
        if (item.id === id && Object.entries(expected).every(([key, value]) => item[key] === value)) return true;
        if (find(item.children)) return true;
      }
      return false;
    };
    return window.__nativeUi && find([...window.__nativeUi.controls, ...window.__nativeUi.sheet, ...window.__nativeUi.sheetActions, ...(window.__nativeUi.sheetFooter || [])]);
  }, { id, expected });
  await page.waitForFunction(() => window.__nativeUi?.sessionStatus === "connected");
  const pointer = (target, phase, x, y, pointerId = 1) => page.evaluate(detail =>
    window.dispatchEvent(new CustomEvent("plumelink-native-pointer", { detail })),
  { target, phase, pointerId, x, y, width: 280, primary: pointerId === 1 });
  if (connectionRecovery) {
    const { verifyConnectionRecovery } = await import('./verify-connection-recovery.mjs');
    await verifyConnectionRecovery({ page, context, fire, control, commands, errors, output });
  } else if (shortcutPlans) {
    const checks = [];
    let revision = 100;
    const snapshot = () => page.evaluate(() => JSON.parse(window.PlumeLinkAndroid.getMobilePreferences()));
    const meaningful = () => commands.filter(command => command.type !== "release");
    for (const language of ["zh-CN", "zh-TW", "en"]) {
      const dictionary = language === "zh-CN" ? {} : JSON.parse(await readFile(resolve("shared/locales", language === "en" ? "en.json" : "zh-Hant.json"), "utf8"));
      const tr = key => dictionary[key] || key;
      await fire("nav-settings"); await fire("language"); await fire("language" + language);
      await fire("shortcut-preset"); await fire("shortcut-presetediting"); await fire("icon-0"); await fire("icon-0heart");
      const original = (await snapshot()).shortcuts;
      await fire("nav-remote"); await fire("more");
      if (process.argv.includes("--capture-native-sheets")) await writeFile(resolve(output, `more-${language}.json`), JSON.stringify(await page.evaluate(() => window.__nativeUi)));
      await fire("shortcut-plans-more");
      await control("plan-create", { label: tr("保存当前组合") });
      if (process.argv.includes("--capture-native-sheets")) await writeFile(resolve(output, `plans-${language}.json`), JSON.stringify(await page.evaluate(() => window.__nativeUi)));
      const before = meaningful().length;
      await fire("plan-create"); await fire("plan-save"); await control("plan-error");
      const name = "Work " + language + " 😀";
      await fire("plan-name", { text: name, revision: revision++ }); await fire("plan-save");
      await control("plan-apply", { label: tr("应用方案") });
      await control("plan-back", { iconOnly: true });
      const saved = (await snapshot()).shortcutPlans.find(item => item.name === name);
      assert.ok(saved); assert.deepEqual(saved.shortcuts, original);
      const stale = await page.evaluate(() => window.__nativeUi.sheet.find(item => item.id === "plan-apply").actionId);
      await fire("plan-back"); await fire("plan-media"); await control("plan-preview-1", { label: tr("播放/暂停") });
      await page.evaluate(id => window.dispatchEvent(new CustomEvent("plumelink-native-ui", { detail: { id } })), stale);
      assert.equal(await page.evaluate(() => window.__nativeUi.sheetKey), "shortcut-plans-preview");
      assert.deepEqual((await snapshot()).shortcuts, original, "an old preview must not apply a new selection");
      await fire("plan-apply"); await control("run-1", { label: tr("播放/暂停") });
      await fire("shortcut-plans-more"); await fire("plan-" + saved.id); await fire("plan-apply");
      await control("run-0", { icon: "heart" }); assert.deepEqual((await snapshot()).shortcuts, original);
      await fire("shortcut-plans-more"); await fire("plan-" + saved.id); await fire("plan-rename");
      await fire("plan-name", { text: name + " saved", revision: revision++ }); await fire("plan-save");
      assert.equal((await snapshot()).shortcutPlans.find(item => item.id === saved.id).name, name + " saved");
      await fire("plan-delete"); await fire("plan-delete-cancel"); assert.ok((await snapshot()).shortcutPlans.some(item => item.id === saved.id));
      await fire("plan-back"); await fire("plan-create"); await fire("plan-name", { text: name + " saved", revision: revision++ }); await fire("plan-save");
      await control("plan-error", { label: tr("已有同名方案，请换一个名称") }); await fire("plan-back");
      await fire("dismiss"); await page.reload(); await page.waitForFunction(() => window.__nativeUi?.sessionStatus === "connected");
      await fire("more"); await fire("shortcut-plans-more"); await fire("plan-" + saved.id);
      await control("plan-preview-0", { icon: "heart" }); await fire("plan-delete"); await fire("plan-delete-confirm");
      assert.ok(!(await snapshot()).shortcutPlans.some(item => item.id === saved.id)); assert.deepEqual((await snapshot()).shortcuts, original);
      await fire("dismiss"); await fire("more"); await control("run-0", { icon: "heart" });
      assert.equal(meaningful().length, before, "plan management must not execute keys or media commands");
      const copies = commands.filter(command => command.type === "combo" && command.key === "c").length;
      await fire("run-0"); await expect.poll(() => commands.filter(command => command.type === "combo" && command.key === "c").length).toBe(copies + 1);
      await fire("dismiss"); checks.push({ language, saved: true, renamed: true, previewed: true, deleted: true, staleIgnored: true, restored: true });
    }
    await fire("nav-settings"); await context.setOffline(true);
    await page.evaluate(() => window.__nativeControlSocket.close());
    await page.waitForFunction(() => window.__nativeUi.sessionStatus !== "connected");
    const offlineCommands = meaningful().length;
    await fire("shortcut-plans"); await fire("plan-browser"); await fire("plan-apply");
    await control("slot-0", { detail: "Previous tab" });
    assert.deepEqual((await snapshot()).shortcuts.map(item => item.action), ["previoustab", "nexttab", "refresh"]);
    await context.setOffline(false); await page.evaluate(() => window.dispatchEvent(new Event("online")));
    await page.waitForFunction(() => window.__nativeUi.sessionStatus === "connected");
    assert.equal(meaningful().length, offlineCommands, "offline plan changes must not replay remote keys after reconnection");
    await writeFile(resolve(output, "shortcut-plans-results.json"), JSON.stringify({ passed: true, checks, offlineRestoredWithoutCommands: true }, null, 2));
    assert.deepEqual(errors, []); console.log("Shortcut plans passed save, preview, apply, rename, delete and reload in three languages.");
  } else if (microphoneStatus) {
    const actual = await context.request.get(ready.origin + "/api/microphone/status", {
      headers: { Origin: ready.origin, Authorization: "Bearer " + credentials.token, ...fixtureUsbHeaders },
      maxRedirects: 0,
    });
    assert.equal(actual.status(), 200);
    const initial = await actual.json();
    assert.equal(initial.allowed, true); assert.equal(initial.owned, false); assert.equal(initial.gainDb, 6);
    let reads = 0, deniedCode = 0;
    const signal = { active: true, error: "", receivedFrames: 8, receivedSignalFrames: 4, renderedSignalSamples: 80,
      queuedMs: 40, targetMs: 40, jitterMs: 2, underruns: 0, droppedFrames: 0 };
    let snapshot = { configured: false, allowed: true, output: "", gainDb: 6, owned: false, busy: false, status: { ...signal, active: false } };
    await page.route("**/api/microphone/status", async route => {
      reads++;
      if (deniedCode === 403) await route.fulfill({ status: 403, body: "forbidden" });
      else await route.fulfill({ json: deniedCode === 200 ? { ...snapshot, allowed: false, output: "Private output" } : snapshot });
    });
    const checks = [];
    for (const language of ["zh-CN", "zh-TW", "en"]) {
      const dictionary = language === "zh-CN" ? {} : JSON.parse(await readFile(resolve("shared/locales", language === "en" ? "en.json" : "zh-Hant.json"), "utf8"));
      const tr = key => dictionary[key] || key;
      await fire("nav-settings"); await fire("language"); await fire("language" + language);
      snapshot = { ...snapshot, configured: false, owned: false, busy: false, status: { ...signal, active: false } };
      await fire("microphone-status"); await control("microphone-summary", { label: tr("电脑尚未配置音频设备") });
      assert.ok(await page.evaluate(() => window.__nativeUi.sheet.some(item => item.id === "microphone-guidance" && item.label.includes("→"))));
      snapshot = { ...snapshot, configured: true, output: "Synthetic diagnostic output", busy: true, status: { ...signal, error: "Other private error" } };
      await fire("microphone-refresh"); await control("microphone-summary", { label: tr("另一台手机正在使用麦克风通道") });
      assert.ok(!await page.evaluate(() => JSON.stringify(window.__nativeUi.sheet).includes("Other private error")));
      snapshot = { ...snapshot, busy: false, owned: true, status: { ...signal, receivedSignalFrames: 0 } };
      await fire("microphone-refresh"); await control("microphone-summary", { label: tr("已收到音频，暂未检测到声音") });
      snapshot = { ...snapshot, status: { ...signal } };
      await fire("microphone-refresh"); await control("microphone-summary", { label: tr("声音已送往电脑音频设备") });
      await fire("microphone-details"); await control("microphone-buffer", { label: tr("播放缓冲：{0} ms").replace("{0}", "40") });
      if (language === "zh-CN") {
        await page.evaluate(() => window.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: {
          id: 707, phase: "error", locked: false, snapshot: true, message: "手机麦克风准备超时，请重新开启",
        } })));
        await control("microphone-issue"); await fire("dismiss"); await fire("microphone-status");
        await control("microphone-issue"); await fire("microphone-dismiss-issue");
        await page.waitForFunction(() => !window.__nativeUi.sheet.some(item => item.id === "microphone-issue"));
      }
      snapshot = { ...snapshot, status: { ...signal, error: "音频设备启动超时" } };
      await fire("microphone-refresh"); await control("microphone-computer-error");
      snapshot = { ...snapshot, status: { ...signal } };
      await fire("microphone-refresh"); await control("microphone-summary", { label: tr("声音已送往电脑音频设备") });
      await fire("dismiss"); const stoppedAt = reads; await page.waitForTimeout(2200); assert.equal(reads, stoppedAt, "closed status sheet kept polling");
      checks.push({ language, configurationHelp: true, otherStreamHidden: true, silentAudioIsValid: true, persistentIssue: language === "zh-CN", currentComputerError: true, closedPollingStopped: true });
    }
    assert.equal(await page.evaluate(() => window.__nativeCalls.filter(([name]) => name === "mic-start").length), 0, "reading status started capture");
    await fire("microphone-status"); await control("microphone-output");
    for (const status of [403, 200]) {
      deniedCode = status; await fire("microphone-refresh");
      await control("microphone-summary", { label: "Microphone access not granted" });
      assert.ok(!await page.evaluate(() => window.__nativeUi.sheet.some(item => ["microphone-output", "microphone-gain"].includes(item.id))));
      const atDenial = reads; await page.waitForTimeout(2200); assert.equal(reads, atDenial, "denied status kept polling");
      deniedCode = 0; await fire("microphone-refresh"); await control("microphone-output");
    }
    await fire("dismiss");
    await fire("nav-remote"); await pointer("mic", "down", 80, 150);
    await page.waitForFunction(() => window.__nativeUi.microphone?.recording === true);
    await pointer("mic", "move", 80, 60); await page.waitForFunction(() => window.__lockedMic === true); await pointer("mic", "up", 80, 60);
    const capture = await page.evaluate(() => window.__activeMic);
    await fire("more"); await fire("microphone-status-more"); await control("microphone-stop");
    assert.equal(await page.evaluate(() => window.__activeMic), capture, "status sheet replaced locked capture");
    await fire("microphone-stop"); await page.waitForFunction(() => window.__activeMic === null);
    await fire("dismiss");
    await writeFile(resolve(output, "microphone-status-results.json"), JSON.stringify({ checks, reads, lockedCapturePreserved: true, statusDoesNotStartCapture: true }, null, 2));
    console.log("Microphone status and help passed through the production native bridge in three languages.");
  } else if (windowLive) {
    let rows = [], reads = 0, holdNext = false, release;
    const queryWindows = async text => {
      const revision = await page.evaluate(() => window.__nativeUi.controls.find(item => item.id === "windows-search").children.find(item => item.id === "window-filter").revision);
      await fire("window-filter", { text, revision: revision + 1 }); await control("window-filter", { value: text });
    };
    await page.route("**/api/windows", async route => {
      reads++;
      const snapshot = structuredClone(rows);
      if (holdNext) { holdNext = false; await new Promise(resolve => { release = resolve; }); }
      await route.fulfill({ json: { windows: snapshot } }).catch(() => {});
    });
    const checks = [];
    for (const language of ["zh-CN", "zh-TW", "en"]) {
      await fire("nav-settings"); await fire("language"); await fire("language" + language); await fire("nav-remote");
      rows = [{ hwnd: 41, title: "Original", app: "editor.exe" }, { hwnd: 42, title: "Closed later", app: "notes.exe" }];
      const entryReads = reads;
      await fire("more"); await fire("windows"); await control("window-41", { label: "Original", enabled: true });
      assert.equal(reads, entryReads + 1, "navigation and sheet dismissal must not issue duplicate initial reads");
      await fire("window-interaction", true);
      const oldAction = await page.evaluate(() => window.__nativeUi.controls.find(item => item.id === "window-42").actionId);
      const hiddenAction = await page.evaluate(() => window.__nativeUi.controls.find(item => item.id === "window-41").actionId);
      let accidentalSwitches = 0;
      await page.route("**/api/windows/switch", route => { accidentalSwitches++; return route.fulfill({ json: { switched: true } }); });
      await fire("window-search");
      await control("window-search", { icon: "close" });
      assert.equal(await page.evaluate(() => window.__nativeUi.controls.find(item => item.id === "window-actions").children.find(item => item.id === "window-search").selected), undefined);
      await control("window-filter", { label: { "zh-CN": "搜索窗口", "zh-TW": "搜尋視窗", en: "Search windows" }[language] });
      await queryWindows("ｎＯＴＥＳ");
      assert.deepEqual(await page.evaluate(() => window.__nativeUi.controls.filter(item => item.kind === "window").map(item => item.id)), ["window-42"]);
      await page.evaluate(id => window.dispatchEvent(new CustomEvent("plumelink-native-ui", { detail: { id } })), hiddenAction);
      await page.waitForTimeout(100); assert.equal(accidentalSwitches, 0, "a filtered row must lose its action");
      await queryWindows("not present");
      await control("windows-no-match", { label: { "zh-CN": "没有匹配的窗口", "zh-TW": "沒有符合的視窗", en: "No matching windows" }[language] });
      await fire("clear-window-filter"); await control("window-41", { enabled: true });
      assert.equal(await page.evaluate(() => window.plumeLinkBack()), true);
      await page.waitForFunction(() => window.__nativeUi.destination === "windows" && !window.__nativeUi.controls.some(item => item.id === "windows-search"));
      await fire("window-search"); await control("window-filter"); await queryWindows("editor");
      assert.equal(reads, entryReads + 1, "local search must not request a new window catalog");
      rows = [{ hwnd: 43, title: "New window", app: "new.exe" }, { hwnd: 41, title: "Updated", app: "editor.exe", minimized: true }];
      holdNext = true;
      await fire("window-interaction", false);
      await expect.poll(() => typeof release, { timeout: 6000 }).toBe("function");
      await fire("window-interaction", true); release(); release = undefined;
      await page.waitForTimeout(200);
      await control("window-41", { label: "Original", enabled: true });
      await control("window-refresh", { enabled: true });
      await fire("window-interaction", false); await control("window-41", { label: "Updated", enabled: true });
      await control("window-filter", { value: "editor" });
      assert.deepEqual(await page.evaluate(() => window.__nativeUi.controls.filter(item => item.kind === "window").map(item => item.id)), ["window-41"]);
      await fire("clear-window-filter"); await control("window-43", { enabled: true });
      await fire("window-search");
      assert.deepEqual(await page.evaluate(() => window.__nativeUi.controls.filter(item => item.kind === "window").map(item => item.id)), ["window-41", "window-43"]);
      const detail = await page.evaluate(() => window.__nativeUi.controls.find(item => item.id === "window-41").detail);
      assert.ok(detail.startsWith("editor.exe · "), "minimized state was not localized");
      await page.evaluate(id => window.dispatchEvent(new CustomEvent("plumelink-native-ui", { detail: { id } })), oldAction);
      await page.waitForTimeout(100); assert.equal(accidentalSwitches, 0, "a removed row must lose its action");
      await fire("window-interaction", true); const heldReads = reads;
      await page.waitForTimeout(3200); assert.equal(reads, heldReads, "polling continued during a native gesture");
      await fire("window-interaction", false); await fire("back-remote");
      await page.waitForFunction(() => window.__nativeUi.destination === "remote");
      const leftReads = reads; await page.waitForTimeout(3200); assert.equal(reads, leftReads, "polling continued off the window page");
      checks.push({ language, searchByAppAndWidth: true, staleFilteredTapRejected: true, noMatchAndClear: true, backCollapsesSearch: true,
        queryRetainedDuringRefresh: true, bufferedDuringTouch: true, survivingOrderStable: true, closedRowUnbound: true, backgroundStopped: true, detail });
      await page.unroute("**/api/windows/switch");
    }
    assert.deepEqual(errors, []);
    await writeFile(resolve(output, "window-live-results.json"), JSON.stringify({ passed: true, reads, checks, nativeEffectsSimulated: true }, null, 2));
    console.log("Live window picker passed through the production native bridge in three languages.");
  } else if (launchErrors) {
    const entryId = "a".repeat(32), checks = [];
    await page.route("**/api/applications/icons", route => route.fulfill({ json: { icons: [] } }));
    await fire("nav-settings"); await fire("slot-0"); await fire("slot-0applications");
    for (const language of ["zh-CN", "zh-TW", "en"]) {
      await fire("language"); await fire("language" + language);
      let releaseLookup, calls = 0, available = true;
      const lookupGate = new Promise(resolve => { releaseLookup = resolve; });
      await page.route("**/api/applications", async route => {
        const call = ++calls;
        if (call === 2) { available = false; await lookupGate; }
        await route.fulfill({ json: { applications: [{ id: entryId, name: "Missing App", available }], launchJobs: true } }).catch(() => {});
      });
      await fire("nav-remote"); await fire("more"); await fire("run-0");
      await control("launch-" + entryId, { enabled: true });
      const previousSockets = frames.sockets;
      await fire("launch-" + entryId);
      await control("applications-error", { kind: "notice", tone: "error" });
      await expect.poll(() => calls).toBeGreaterThanOrEqual(2);
      await control("applications-refresh", { enabled: true });
      const original = await page.evaluate(() => window.__nativeUi.sheet.find(item => item.id === "applications-error").label);
      assert.equal(original.split("\n").length, 1, "a confirmed failed job must not become an uncertain QFT failure");
      releaseLookup();
      await control("launch-" + entryId, { enabled: false });
      const final = await page.evaluate(() => window.__nativeUi.sheet.find(item => item.id === "applications-error").label);
      assert.ok(final.startsWith(original), "availability must not replace the command result, including after reconnect");
      assert.ok(!await page.evaluate(() => window.__nativeUi.sheet.some(item => item.id === "applications-notice")));
      assert.equal(frames.sockets, previousSockets, "a known launch failure must not close the input lease");
      await page.waitForFunction(() => window.__nativeUi.sessionStatus === "connected");
      await control("applications-refresh", { enabled: true });
      available = true;
      await fire("applications-refresh");
      await control("launch-" + entryId, { enabled: true });
      await page.waitForFunction(() => !window.__nativeUi.sheet.some(item => item.id === "applications-error"));
      checks.push({ language, original, final, pendingClearedBeforeLookup: true, explicitRefreshRecovered: true });
      await fire("dismiss"); await fire("nav-settings");
      await page.unroute("**/api/applications");
    }
    const launches = commands.filter(command => command.type === "launch");
    assert.equal(launches.length, 3, "each explicit launch admits one job, without automatic resubmission");
    assert.equal(new Set(launches.map(command => command.request)).size, 3);
    assert.ok(launches.every(command => /^[a-f0-9]{32}$/.test(command.request)));
    assert.deepEqual(errors, []);
    await writeFile(resolve(output, "launcher-results.json"), JSON.stringify({ passed: true, checks, productionLaunchQueue: true, nativeEffectsSimulated: true }, null, 2));
    console.log("Native queued launcher failures passed using real QFT2/WSS in three languages.");
  } else if (nativeUi) {
    await page.waitForFunction(() => window.__nativeUi?.connection.label.startsWith("已连接"));
    assert.equal(await page.locator(".app-shell").count(), 0);
    const remoteLayout = await page.evaluate(() => window.__nativeUi.controls.map(item => item.id));
    await page.evaluate(() => window.__nativeControlSocket.close());
    await page.waitForFunction(() => window.__nativeUi.connection.hint === "正在重连…");
    await control("offline-change", { enabled: true });
    await fire("offline-change");
    assert.ok(await page.evaluate(() => window.__nativeCalls.some(([name]) => name === "computers")));
    assert.deepEqual(await page.evaluate(() => window.__nativeUi.controls.map(item => item.id)), remoteLayout);
    await fire("connection-chip");
    await page.waitForFunction(() => window.__nativeUi.sheetKey === "connection");
    await control("pc-status");
    await page.waitForFunction(() => window.__nativeUi.connection.hint === "已连接");
    await control("focus", { enabled: true });
    await fire("dismiss");
    await fire("enter");
    await fire("left");
    await fire("quieter");
    await fire("play");
    await fire("pad-side");
    await page.waitForFunction(() => window.__nativeUi.side);
    await fire("focus");
    await page.waitForFunction(() => window.__nativeUi.nav.length === 0);
    await fire("focus");
    await pointer("pad", "down", 80, 80);
    await pointer("pad", "move", 100, 94);
    await page.waitForTimeout(45);
    await pointer("pad", "up", 100, 94);
    await fire("drag", true);
    await pointer("pad", "down", 80, 80);
    await control("pad", { status: "拖拽中", detail: "松手结束拖拽", hint: "" });
    assert.equal(await page.evaluate(() => window.__nativeUi.feedback), "");
    await pointer("pad", "up", 80, 80);
    await control("pad", { status: "可操作", detail: "滑动，移动指针" });
    assert.equal(await page.evaluate(() => window.__nativeUi.feedback), "");
    await expect.poll(() => commands.some(c => c.type === "buttonstate" && c.button === "left" && c.down === false)).toBe(true);
    const beforeSwipe = commands.length;
    await page.evaluate(() => {
      const points = [2, 7, 4].map((pointerId, i) => ({ pointerId, x: 80 + 30 * i, y: 100 }));
      const dispatch = detail => window.dispatchEvent(new CustomEvent("plumelink-native-pointer", { detail: { target: "pad", ...detail } }));
      for (const [i, p] of points.entries()) dispatch({ phase: "down", ...p, primary: i === 0, width: 280 });
      for (const dx of [50, -50]) dispatch({ phase: "move-batch", contacts: points.map(p => ({ ...p, x: p.x + dx })) });
      for (const p of points) dispatch({ phase: "up", ...p, x: p.x - 50, width: 280 });
    });
    await control("pad", { selected: false });
    await fire("enter");
    await expect.poll(() => commands.slice(beforeSwipe).some(c => c.type === "key" && c.key === "enter")).toBe(true);
    assert.ok(!commands.slice(beforeSwipe).some(c => ["appswitch", "click", "scroll", "zoom", "buttonstate"].includes(c.type)));
    const micState = expected => page.waitForFunction(values => {
      const state = window.__nativeUi?.microphone;
      return state && Object.entries(values).every(([key, value]) => state[key] === value);
    }, expected);
    await pointer("mic", "down", 80, 150);
    await pointer("mic", "up", 80, 150);
    await micState({ expanded: false, locked: false, progress: 0 });
    assert.equal(await page.evaluate(() => window.__nativeCalls.filter(([name]) => name === "mic-start").length), 0);
    await pointer("mic", "down", 80, 150);
    await micState({ expanded: true, recording: true, locked: false });
    await pointer("mic", "up", 80, 150);
    await micState({ expanded: false, recording: false, locked: false });
    await pointer("mic", "down", 80, 150);
    await micState({ expanded: true, recording: true, locked: false });
    await pointer("mic", "move", 80, 60);
    await page.waitForFunction(() => {
      const state = window.__nativeUi.microphone;
      return state.intent === "lock" && state.progress > 0 && state.progress < 100;
    });
    await page.waitForFunction(() => window.__lockedMic === true);
    await pointer("mic", "up", 80, 60);
    await micState({ expanded: false, recording: true, locked: true });
    const lockedMic = await page.evaluate(() => window.__activeMic);
    await pointer("mic", "cancel", 80, 60);
    await page.waitForTimeout(80);
    assert.equal(await page.evaluate(() => window.__nativeUi.microphone.locked), true, "touch cancellation must preserve a locked microphone");
    await pointer("mic", "down", 80, 150);
    await pointer("mic", "lost", 80, 150);
    await micState({ expanded: false, recording: true, locked: true, progress: 0 });
    assert.equal(await page.evaluate(() => window.__activeMic), lockedMic);
    await fire("nav-settings");
    await control("microphone-mode", { enabled: false, detail: "标准收音" });
    await control("microphone-mode-help", { label: "停止收音后可更改收音模式" });
    await fire("microphone-mode");
    assert.ok(!await page.evaluate(() => window.__nativeUi.sheetKey));
    await fire("nav-files");
    await page.waitForFunction(() => window.__nativeUi.destination === "files");
    await micState({ expanded: false, recording: true, locked: true });
    const stoppedMic = await page.evaluate(() => window.__activeMic);
    await fire("mic-stop");
    await page.waitForFunction(() => window.__activeMic === null);
    await fire("nav-settings");
    await control("microphone-mode", { enabled: true });
    await fire("microphone-mode"); await fire("microphone-modevoice");
    await control("microphone-mode", { detail: "语音增强" });
    assert.equal(await page.evaluate(() => JSON.parse(window.PlumeLinkAndroid.getMobilePreferences()).microphoneMode), "voice");
    await fire("nav-remote");
    const staleMicSnapshot = phase => page.evaluate(({ id, phase }) => window.dispatchEvent(new CustomEvent("plumelink-microphone", {
      detail: { id, phase, locked: phase === "live", snapshot: true, level: .9 },
    })), { id: stoppedMic, phase });
    await staleMicSnapshot("live");
    await micState({ expanded: false, recording: false, locked: false, level: 0 });
    await pointer("mic", "down", 80, 150);
    await micState({ expanded: true, recording: true, locked: false });
    const currentMic = await page.evaluate(() => window.__activeMic);
    assert.notEqual(currentMic, stoppedMic);
    await staleMicSnapshot("idle"); await staleMicSnapshot("live");
    await micState({ expanded: true, recording: true, locked: false });
    await pointer("mic", "up", 80, 150);
    await micState({ expanded: false, recording: false, locked: false, level: 0 });
    assert.equal(await page.evaluate(() => window.__nativeCalls.filter(([name]) => name === "mic-stop").at(-1)[1]), currentMic);
    await pointer("mic", "down", 80, 150);
    await micState({ expanded: true, recording: true, locked: false });
    await pointer("mic", "lost", 80, 150, 2);
    await micState({ expanded: true, recording: true, locked: false });
    await pointer("mic", "cancel", 80, 150);
    await micState({ expanded: false, recording: false, locked: false, level: 0 });
    assert.equal(await page.evaluate(() => window.__activeMic), null, "touch cancellation must stop an unlocked microphone");
    await fire("more");
    await page.waitForFunction(() => window.__nativeUi.sheetKey === "more");
    const windowIconBytes = Buffer.alloc(58);
    windowIconBytes.write("BM"); windowIconBytes.writeUInt32LE(58,2); windowIconBytes.writeUInt32LE(54,10); windowIconBytes.writeUInt32LE(40,14);
    windowIconBytes.writeInt32LE(1,18); windowIconBytes.writeInt32LE(1,22); windowIconBytes.writeUInt16LE(1,26); windowIconBytes.writeUInt16LE(32,28);
    const windowIcon = "data:image/bmp;base64," + windowIconBytes.toString("base64");
    const fixtureWindow = { hwnd: 42, title: "Editor", app: "editor.exe", minimized: true, icon: windowIcon };
    await page.route("**/api/windows", route => route.fulfill({ json: { windows: [fixtureWindow,
      { hwnd: 43, title: "Fallback", app: {}, icon: "https://example.com/icon.svg" },
      { hwnd: 42, title: "Duplicate" },
    ] } }));
    let switchedWindow;
    await page.route("**/api/windows/switch", route => {
      switchedWindow = route.request().postDataJSON()?.hwnd;
      return route.fulfill({ json: { switched: true } });
    });
    await fire("windows");
    await page.waitForFunction(() => window.__nativeUi.destination === "windows");
    await control("window-42", { label: "Editor", detail: "editor.exe · 已最小化", icon: windowIcon });
    await control("window-43", { label: "Fallback", detail: "", icon: "monitor" });
    assert.equal(await page.evaluate(() => window.__nativeUi.controls.filter(item => item.id === "window-42").length), 1);
    let reads = 0;
    let unavailable = true;
    await page.route("**/api/windows", route => {
      reads++;
      return unavailable ? route.fulfill({ status: 503, json: { error: "窗口列表超时" } })
        : route.fulfill({ json: { windows: [fixtureWindow] } });
    });
    await expectHttpFailure("/api/windows", 503, () => fire("window-refresh"));
    await control("window-42", { enabled: false });
    await page.waitForFunction(() => window.__nativeUi.feedback === "窗口列表超时");
    assert.ok(!await page.evaluate(() => window.__nativeUi.controls.some(c => c.id === "windows-error")));
    await fire("window-42");
    assert.equal(switchedWindow, undefined, "cached windows cannot be selected after a failed refresh");
    await fire("dismiss-feedback");
    await page.waitForFunction(() => window.__nativeUi.feedback === "");
    await page.waitForFunction(() => window.__nativeUi.controls.find(c => c.id === "window-actions")?.children.some(c => c.id === "window-actions-spacer" && c.label === "请刷新后重试"));
    await control("window-42", { enabled: false });
    unavailable = false;
    await page.evaluate(() => window.__nativeControlSocket.close());
    await control("windows-offline");
    await control("window-42", { enabled: true });
    await page.waitForFunction(() => window.__nativeUi.controls.find(c => c.id === "window-actions")?.children.some(c => c.id === "window-actions-spacer" && c.label === ""));
    assert.equal(reads, 2, "reconnection must fetch a fresh list");
    let finishSwitch;
    await page.route("**/api/windows/switch", route => {
      switchedWindow = route.request().postDataJSON()?.hwnd;
      return new Promise(resolve => { finishSwitch = () => resolve(route.fulfill({ json: { switched: true } })); });
    });
    const beforeSwitch = await page.evaluate(() => window.__nativeUi.controls.map(item => item.id));
    await fire("window-42");
    await control("window-42", { enabled: false, detail: "正在切换…" });
    assert.deepEqual(await page.evaluate(() => window.__nativeUi.controls.map(item => item.id)), beforeSwitch, "pending state cannot insert a row above the list");
    await expect.poll(() => typeof finishSwitch).toBe("function");
    await finishSwitch();
    await page.waitForFunction(() => window.__nativeUi.feedback === "已切换到 Editor");
    await control("window-42", { detail: "editor.exe", enabled: true });
    assert.equal(switchedWindow, 42);
    await page.route("**/api/windows/switch", route => route.fulfill({ status: 409,
      json: { error: "窗口尚未切到前台，请打开任务视图选择窗口" } }));
    await expectHttpFailure("/api/windows/switch", 409, () => fire("window-42"));
    await page.waitForFunction(() => window.__nativeUi.feedback === "窗口尚未切到前台，请打开任务视图选择窗口");
    await control("taskview", { enabled: true });
    const taskViewsBefore = commands.filter(c => c.type === "taskview").length;
    await fire("taskview");
    await page.waitForFunction(() => window.__nativeUi.destination === "remote");
    assert.equal(commands.filter(c => c.type === "taskview").length, taskViewsBefore + 1);
    await fire("keyboard");
    await page.waitForFunction(() => window.__nativeUi.destination === "keyboard");
    await fire("draft", { text: "原生输入", revision: 1 });
    await fire("send");
    await control("text-feedback");
    const previousSend = await page.evaluate(() => {
      const find = items => { for (const item of items || []) { if (item.id === 'send') return item; const child = find(item.children); if (child) return child; } };
      return find(window.__nativeUi.controls).actionId || 'send';
    });
    await fire("back-remote");
    await fire("keyboard");
    await page.waitForFunction(() => window.__nativeUi.destination === "keyboard");
    await fire("draft", { text: "保留新草稿", revision: 2 });
    await control("draft", { value: "保留新草稿" });
    const textsBeforeStaleSend = commands.filter(c => c.type === "text").length;
    await page.evaluate(id => window.dispatchEvent(new CustomEvent("plumelink-native-ui", { detail: { id } })), previousSend);
    await page.waitForTimeout(200);
    assert.equal(commands.filter(c => c.type === "text").length, textsBeforeStaleSend, "a previous keyboard page cannot send the new draft");
    await control("draft", { value: "保留新草稿" });
    await fire("send");
    await control("draft", { value: "" });
    await fire("draft", { text: "再次编辑的草稿", revision: 3 });
    await page.evaluate(() => {
      const socket = window.__nativeControlSocket, receive = socket.onmessage;
      const held = [];
      socket.onmessage = event => {
        if (event.data instanceof ArrayBuffer && event.data.byteLength === 64 && new Uint8Array(event.data)[4] === 6) held.push(event);
        else receive.call(socket, event);
      };
      window.__heldTextAcks = held;
      window.__flushTextAcks = () => {
        socket.onmessage = receive;
        held.splice(0).forEach(event => receive.call(socket, event));
      };
    });
    const textsBeforeEditing = commands.filter(c => c.type === "text").length;
    await fire("send");
    await page.waitForFunction(() => window.__heldTextAcks.length > 0);
    await control("send", { label: "发送中…", enabled: false });
    await fire("draft", { text: "新的内容", revision: 4 });
    await fire("draft", { text: "再次编辑的草稿", revision: 5 });
    await fire("send");
    await page.evaluate(() => window.__flushTextAcks());
    await control("send", { label: "发送文字" });
    assert.equal(await page.evaluate(() => {
      const entry = window.__nativeUi.controls.find(item => item.id === "keyboard-compose");
      return entry.children.find(item => item.id === "keyboard-entry").children.find(item => item.id === "draft").value;
    }), "再次编辑的草稿", "a late text acknowledgement cannot clear a draft edited back to the same contents");
    assert.equal(commands.filter(c => c.type === "text").length, textsBeforeEditing + 1, "editing while a send is pending must not send again");
    await control("send", { enabled: true });
    await fire("send");
    await control("draft", { value: "" });
    assert.equal(commands.filter(c => c.type === "text").length, textsBeforeEditing + 2, "the retained draft can be sent by a new explicit action");
    await fire("back-remote");
    await fire("nav-settings");
    await page.waitForFunction(() => window.__nativeUi.destination === "settings");
    await fire("slot-0");
    await control("shortcut-filter", { label: "搜索操作" });
    await fire("shortcut-filter", { text: "Ｃｔｒｌ Ｃ", revision: 1 });
    await control("slot-0copy", { detail: "Ctrl + C", icon: "layers" });
    assert.ok(!await page.evaluate(() => window.__nativeUi.sheet.some(c => c.id === "slot-0paste")));
    await fire("shortcut-filter", { text: "missing-action", revision: 2 });
    await control("shortcut-empty", { label: "没有匹配的操作" });
    await fire("clear-shortcut-filter");
    await control("slot-0paste", { icon: "keyboard" });
    await fire("slot-0paste");
    await control("icon-0", { icon: "keyboard" });
    await fire("icon-0"); await fire("icon-0heart");
    await fire("slot-0"); await control("shortcut-filter", { value: "" });
    await fire("slot-0copy");
    await control("icon-0", { icon: "heart" });
    await fire("slot-0"); await fire("slot-0custom-key");
    await control("custom-key-save");
    const previousSave = await page.evaluate(() => window.__nativeUi.sheetActions[0].actionId || 'custom-key-save');
    await fire("dismiss");
    await fire("slot-1"); await fire("slot-1custom-key");
    await fire("custom-key-p");
    await page.evaluate(id => window.dispatchEvent(new CustomEvent("plumelink-native-ui", { detail: {id} })), previousSave);
    assert.equal(await page.evaluate(() => window.__nativeUi.sheetKey), 'custom-key-editor', 'an old editor Save must not save or dismiss a different slot');
    await fire("custom-key-save");
    await control("slot-1", {detail:'Ctrl + P'});
    await fire("shortcut-preset");
    await fire("shortcut-presetediting");
    await control("slot-0", { detail: "复制" });
    await control("slot-1", { detail: "粘贴" });
    await control("slot-2", { detail: "撤销" });
    await fire("nav-remote");
    await fire("more");
    await fire("run-0");
    await expect.poll(() => commands.some(c => c.type === "combo" && c.key === "c")).toBe(true);
    await fire("dismiss");
    await fire("nav-settings");
    await fire("shortcut-preset"); await fire("shortcut-presetmedia");
    await control("slot-0", { detail: "上一首" });
    await control("slot-1", { detail: "播放/暂停" });
    await control("slot-2", { detail: "下一首" });
    await fire("nav-remote"); await fire("more");
    for (const [id, action] of [["run-0", "previous"], ["run-1", "playpause"], ["run-2", "next"]]) {
      const count = commands.filter(c => c.type === "media" && c.action === action).length;
      await fire(id);
      await expect.poll(() => commands.filter(c => c.type === "media" && c.action === action).length).toBe(count + 1);
    }
    await fire("dismiss"); await fire("nav-settings");
    await page.evaluate(() => {
      for (const [slot, action, icon] of [["a", "presentation", "heart"], ["b", "fullscreen", "bell"], ["c", "mute", "star"]]) {
        localStorage.setItem("plumelink-shortcut-" + slot, action);
        localStorage.setItem("plumelink-shortcut-icon-" + slot, icon);
      }
      window.PlumeLinkAndroid.setMobilePreferences(JSON.stringify({ ...JSON.parse(window.PlumeLinkAndroid.getMobilePreferences()), shortcuts: [{ action: "presentation", icon: "heart" }, { action: "fullscreen", icon: "bell" }, { action: "mute", icon: "star" }] }));
    });
    await page.reload();
    await page.waitForFunction(() => window.__nativeUi?.sessionStatus === "connected");
    await fire("nav-settings");
    await control("shortcut-preset", { detail: "自定义" });
    await control("slot-0", { detail: "开始放映" });
    await control("slot-1", { detail: "全屏切换" });
    await control("slot-2", { detail: "静音" });
    await fire("nav-remote"); await fire("more");
    await control("run-0", { icon: "heart" });
    await control("run-1", { icon: "bell" });
    await control("run-2", { icon: "star" });
    await fire("dismiss"); await fire("nav-settings");
    await fire("shortcut-preset"); await fire("shortcut-presetslides");
    await control("slot-0", { detail: "上一页" });
    await control("slot-1", { detail: "黑屏切换" });
    await control("slot-2", { detail: "下一页" });
    await fire("nav-remote"); await fire("more");
    for (const [id, key] of [["run-0", "pageup"], ["run-1", "b"], ["run-2", "pagedown"]]) {
      const before = commands.filter(command => command.type === "key" && command.key === key).length;
      await fire(id);
      await expect.poll(() => commands.filter(command => command.type === "key" && command.key === key).length).toBe(before + 1);
    }
    for (const [value, expected] of [["slidecurrent", { type: "combo", modifiers: ["shift"], key: "f5" }], ["presentation", { type: "key", key: "f5" }]]) {
      await fire("dismiss"); await fire("nav-settings"); await fire("slot-0"); await fire("slot-0" + value);
      await fire("nav-remote"); await fire("more");
      const before = commands.filter(command => isDeepStrictEqual(command, expected)).length;
      await fire("run-0");
      await expect.poll(() => commands.filter(command => isDeepStrictEqual(command, expected)).length).toBe(before + 1);
    }
    await fire("dismiss"); await fire("nav-settings");
    await page.evaluate(() => window.__nativeControlSocket.close());
    await control("connection", { detail: "正在重连…" });
    await page.waitForFunction(() => window.__nativeUi.connection.hint === "已连接");
    for (const [locale, settings, language, remote, more, windows, files, receive] of [
      ["zh-CN", "设置", "语言", "遥控", "更多操作", "切换窗口", "文件", "Android 接收目录"],
      ["zh-TW", "設定", "語言", "遙控", "更多操作", "切換視窗", "檔案", "Android 接收目錄"],
      ["en", "Settings", "Language", "Remote", "More actions", "Switch windows", "Files", "Android download folder"],
    ]) {
      await fire("language");
      await page.waitForFunction(() => window.__nativeUi.sheetKey === "language");
      await fire("language" + locale);
      await page.waitForFunction(title => window.__nativeUi.title === title && !window.__nativeUi.sheetKey, settings);
      await control("language", { label: language });
      await control("microphone-mode", { label: { "zh-CN": "收音模式", "zh-TW": "收音模式", en: "Capture mode" }[locale], detail: { "zh-CN": "语音增强", "zh-TW": "語音增強", en: "Speech enhanced" }[locale] });
      await control("hardware-volume", { value: false, label: { "zh-CN": "音量键控制电脑", "zh-TW": "音量鍵控制電腦", en: "Volume keys control PC" }[locale] });
      await fire("nav-remote");
      await page.waitForFunction(title => window.__nativeUi.title === title, remote);
      for (const [message, translated] of [
        ["手机麦克风准备超时，请重新开启", {"zh-CN":"手机麦克风准备超时，请重新开启","zh-TW":"手機麥克風準備逾時，請重新開啟",en:"Phone microphone setup timed out. Start it again."}],
        ["电脑音频设备正在停止，请稍后重试", {"zh-CN":"电脑音频设备正在停止，请稍后重试","zh-TW":"電腦音訊裝置正在停止，請稍後重試",en:"The PC audio device is stopping. Try again shortly."}],
      ]) {
        await page.evaluate(() => window.__holdMicStart = true);
        await fire("mic", true);
        await page.waitForFunction(() => Number.isSafeInteger(window.__activeMic));
        await micState({ recording: false, expanded: true });
        const failedRequest = await page.evaluate(() => window.__activeMic);
        const stopsBeforeError = await page.evaluate(() => window.__nativeCalls.filter(([name]) => name === "mic-stop").length);
        await page.evaluate(({ id, message }) => {
          if (window.__activeMic === id) { window.__activeMic = null; window.__lockedMic = false; }
          window.dispatchEvent(new CustomEvent("plumelink-microphone", { detail: { id, phase: "error", locked: false, message } }));
        }, { id: failedRequest, message });
        await micState({ expanded: false, recording: false, locked: false, level: 0 });
        await page.waitForFunction(message => window.__nativeUi.feedback === message, translated[locale]);
        assert.equal(await page.evaluate(() => window.__activeMic), null, "a startup error cannot leave native capture active");
        assert.equal(await page.evaluate(() => window.__nativeCalls.filter(([name]) => name === "mic-stop").length), stopsBeforeError, "a native terminal must not send another Stop and erase the service error");
        await page.evaluate(() => window.__holdMicStart = false);
      }
      await fire("more");
      await page.waitForFunction(title => window.__nativeUi.sheetTitle === title, more);
      await fire("windows");
      await page.waitForFunction(title => window.__nativeUi.destination === "windows" && window.__nativeUi.title === title, windows);
      await fire("back-remote");
      await fire("nav-files");
      await page.waitForFunction(title => window.__nativeUi.title === title, files);
      await control("receive", { label: receive });
      await fire("folder");
      await page.waitForFunction(title => window.__nativeUi.sheetKey === "folder" && window.__nativeUi.sheetTitle === title, receive);
      await control("folder-path", { label: "下载 / PlumeLink" });
      await fire("dismiss");
      await fire("nav-settings");
      await page.waitForFunction(title => window.__nativeUi.title === title, settings);
    }
    await fire("control-mode");
    await fire("control-modedpad");
    await fire("nav-remote");
    await control("dpad-center", { enabled: true });
    await fire("dpad-up");
    await expect.poll(() => commands.some(c => c.type === "key" && c.key === "up")).toBe(true);
    await fire("dpad-esc");
    await expect.poll(() => commands.some(c => c.type === "key" && c.key === "escape")).toBe(true);
    await fire("focus");
    await page.waitForFunction(() => window.__nativeUi.keepDialPosition);
    await fire("focus");
    await fire("nav-settings");
    await fire("theme");
    await fire("themedark");
    await page.waitForFunction(() => window.__nativeUi.dark === true && window.__nativeUi.sheetKey === null);
    await fire("sensitivity", 2.1);
    await control("sensitivity", { value: 2.1 });
    await fire("alive", false);
    await control("alive", { value: false });
    await fire("alive", true);
    await fire("slot-0");
    await fire("slot-0mute");
    await control("slot-0", { detail: "Mute" });
    await fire("icon-0");
    await fire("icon-0heart");
    await control("icon-0", { icon: "heart" });
    await page.route("**/api/applications", route => route.fulfill({ json: { applications: [{ id: "a".repeat(32), name: "Sample App" }] } }));
    const iconBytes = Buffer.alloc(58);
    iconBytes.write("BM"); iconBytes.writeUInt32LE(58, 2); iconBytes.writeUInt32LE(54, 10); iconBytes.writeUInt32LE(40, 14);
    iconBytes.writeInt32LE(1, 18); iconBytes.writeInt32LE(1, 22); iconBytes.writeUInt16LE(1, 26); iconBytes.writeUInt16LE(32, 28);
    const applicationIcon = "data:image/bmp;base64," + iconBytes.toString("base64");
    let releaseIcons;
    const iconGate = new Promise(resolve => { releaseIcons = resolve; });
    await page.route("**/api/applications/icons", async route => {
      await iconGate;
      await route.fulfill({ json: { icons: [{ id: "a".repeat(32), icon: applicationIcon }] } });
    });
    await fire("slot-0"); await fire("slot-0applications");
    await fire("nav-remote"); await fire("more"); await fire("run-0");
    await control("applications-refresh", { icon: "refresh", iconOnly: true, enabled: true });
    assert.ok(!await page.evaluate(() => window.__nativeUi.sheet.some(item => item.id === "applications-refresh")));
    await control("launch-" + "a".repeat(32), { enabled: true });
    assert.equal(await page.evaluate(() => window.__nativeUi.sheet.find(item => item.id === "launch-" + "a".repeat(32)).icon), undefined);
    releaseIcons();
    await control("launch-" + "a".repeat(32), { enabled: true, icon: applicationIcon });
    let releaseApplications;
    const applicationsGate = new Promise(resolve => { releaseApplications = resolve; });
    await page.unroute("**/api/applications");
    await page.route("**/api/applications", async route => {
      await applicationsGate;
      await route.fulfill({ json: { applications: [{ id: "a".repeat(32), name: "Sample App", available: true }] } });
    });
    await fire("applications-refresh");
    await control("launch-" + "a".repeat(32), { enabled: false });
    releaseApplications();
    await control("launch-" + "a".repeat(32), { enabled: true });
    await page.unroute("**/api/applications");
    await page.route("**/api/applications", route => route.fulfill({ status: 503, json: { error: "Could not check application status. Try again." } }));
    await expectHttpFailure("/api/applications", 503, () => fire("applications-refresh"));
    await control("applications-error", { kind: "notice", tone: "error" });
    await control("launch-" + "a".repeat(32), { enabled: false });
    await fire("launch-" + "a".repeat(32));
    assert.equal(commands.filter(command => command.type === "launch").length, 0, "failed refresh must not revive cached targets");
    await page.unroute("**/api/applications");
    await page.route("**/api/applications", route => route.fulfill({ json: { applications: [{ id: "a".repeat(32), name: "Renamed App", available: true }] } }));
    await fire("applications-refresh");
    await control("launch-" + "a".repeat(32), { enabled: true, label: "Renamed App" });
    await fire("launch-" + "a".repeat(32));
    await expect.poll(() => commands.filter(command => command.type === "launch").length).toBe(1);
    await control("applications-notice", { kind: "notice", tone: "success", label: "Requested launch of Renamed App" });
    const launcherSockets = frames.sockets;
    await page.evaluate(() => window.__nativeControlSocket.close());
    await expect.poll(() => frames.sockets).toBeGreaterThan(launcherSockets);
    await control("applications-refresh", { enabled: true });
    await control("applications-notice", { kind: "notice", tone: "success", label: "Requested launch of Renamed App" });
    assert.equal(commands.filter(command => command.type === "launch").length, 1);
    await page.unroute("**/api/applications/icons");
    await page.route("**/api/applications/icons", route => route.fulfill({ status: 404, json: { error: "old PC" } }));
    await expectHttpFailure("/api/applications/icons", 404, () => fire("applications-refresh"));
    await control("launch-" + "a".repeat(32), { enabled: true, icon: applicationIcon });
    await page.unroute("**/api/applications");
    await page.route("**/api/applications", route => route.fulfill({ json: { applications: [
      { id: "d".repeat(32), name: "Project documents", kind: "folder", available: true },
      { id: "e".repeat(32), name: "Missing folder", kind: "folder", available: false },
      { id: "f".repeat(32), name: "Unsupported entry", kind: "shell", available: true },
    ] } }));
    await fire("applications-refresh");
    await control("launch-" + "d".repeat(32), { enabled: true, icon: "folder" });
    await control("launch-" + "e".repeat(32), { enabled: false, icon: "folder", detail: "The folder was moved or deleted. Add it again on your PC." });
    assert.ok(!await page.evaluate(() => window.__nativeUi.sheet.some(item => item.id === "launch-" + "f".repeat(32))));
    await fire("launch-" + "e".repeat(32));
    assert.equal(commands.filter(command => command.type === "launch").length, 1, "missing folders do not send launch commands");
    await page.evaluate(id => {
      const actionId = window.__nativeUi.sheet.find(item => item.id === id)?.actionId || id;
      for (let i = 0; i < 2; i++) window.dispatchEvent(new CustomEvent("plumelink-native-ui", { detail: { id: actionId } }));
    }, "launch-" + "d".repeat(32));
    await control("applications-notice", { kind: "notice", tone: "success", label: "Requested to open Project documents" });
    assert.deepEqual(commands.filter(command => command.type === "launch").at(-1), { type: "launch", id: "d".repeat(32) });
    assert.equal(commands.filter(command => command.type === "launch").length, 2, "pending duplicate folder taps execute once");
    await page.unroute("**/api/applications/icons");
    await page.route("**/api/applications/icons", route => route.fulfill({ json: { icons: [] } }));
    const catalog = Array.from({ length: 32 }, (_, index) => ({
      id: (index + 1).toString(16).padStart(32, "0"),
      name: index === 0 ? "Ｃａｆé Ｔｏｏｌ alpha" : index === 1 ? "项目 资料 Archive" : `Utility ${index + 1} with a long descriptive name`,
      kind: index === 1 ? "folder" : "application", available: index !== 2,
    }));
    let currentCatalog = catalog;
    let failCatalog = false;
    await page.unroute("**/api/applications");
    await page.route("**/api/applications", route => route.fulfill(failCatalog
      ? { status: 503, json: { error: "Catalog unavailable" } }
      : { json: { applications: currentCatalog } }));
    await fire("applications-refresh");
    await control("application-filter", { label: "Search apps or folders", value: "" });
    const oldLaunch = await page.evaluate(() => window.__nativeUi.sheet.find(item => item.id?.startsWith("launch-")).actionId);
    await fire("application-filter", { text: "ＣＡＦＥ\u0301 tool", revision: 1 });
    await control("launch-" + catalog[0].id, { enabled: true });
    assert.equal(await page.evaluate(() => window.__nativeUi.sheet.filter(item => item.id?.startsWith("launch-")).length), 1, "search normalizes width/case/accents and matches every word");
    await fire("application-filter", { text: "no matching entry", revision: 2 });
    await control("applications-no-match", { label: "No matching shortcuts" });
    await page.evaluate(id => window.dispatchEvent(new CustomEvent("plumelink-native-ui", { detail: { id } })), oldLaunch);
    assert.equal(commands.filter(command => command.type === "launch").length, 2, "filtered-out old buttons cannot launch an application");
    await fire("application-filter", { text: "资料 项目", revision: 3 });
    await control("launch-" + catalog[1].id, { enabled: true, icon: "folder" });
    const launchCount = commands.filter(command => command.type === "launch").length;
    await fire("launch-" + catalog[1].id);
    await expect.poll(() => commands.filter(command => command.type === "launch").length).toBe(launchCount + 1);
    await control("applications-notice", { label: "Requested to open " + catalog[1].name });
    assert.deepEqual(commands.filter(command => command.type === "launch").at(-1), { type: "launch", id: catalog[1].id }, "a filtered folder retains its original launch ID");
    currentCatalog = [catalog[1]];
    await fire("applications-refresh");
    await control("application-filter", { value: "资料 项目" });
    await control("launch-" + catalog[1].id, { enabled: true });
    assert.equal(await page.evaluate(() => window.__nativeUi.sheet.filter(item => item.id?.startsWith("launch-")).length), 1);
    await fire("clear-application-filter");
    await control("application-filter", { value: "" });
    currentCatalog = catalog;
    await fire("applications-refresh");
    await control("launch-" + catalog[2].id, { enabled: false });
    await fire("application-filter", { text: "资料", revision: 4 });
    const searchSockets = frames.sockets;
    await page.evaluate(() => window.__nativeControlSocket.close());
    await control("application-filter", { value: "资料", enabled: false });
    await expect.poll(() => frames.sockets).toBeGreaterThan(searchSockets);
    await control("application-filter", { value: "资料", enabled: true });
    await control("launch-" + catalog[1].id, { enabled: true });
    assert.equal(commands.filter(command => command.type === "launch").length, launchCount + 1, "reconnecting a filtered catalog must not relaunch its target");
    failCatalog = true;
    await expectHttpFailure("/api/applications", 503, () => fire("applications-refresh"));
    await fire("clear-application-filter");
    await control("launch-" + catalog[0].id, { enabled: false });
    await fire("launch-" + catalog[0].id);
    assert.equal(commands.filter(command => command.type === "launch").length, launchCount + 1, "clearing a filter after failed refresh must not revive cached launch targets");
    failCatalog = false;
    await fire("applications-refresh");
    await control("launch-" + catalog[0].id, { enabled: true });
    await fire("application-filter", { text: "资料", revision: 5 });
    await fire("dismiss"); await fire("more"); await fire("run-0");
    await control("application-filter", { value: "" });
    await control("launch-" + catalog[0].id, { enabled: true });
    await fire("dismiss"); await fire("nav-settings");
    await fire("connection");
    await fire("diagnostics");
    await control("permissions");
    await fire("dismiss");
    await fire("help");
    await control("input-detail");
    await fire("dismiss");
    await page.route("**/api/files", route => route.fulfill({ json: { files: [{ id: 99, name: "test.bin", size: 128 }] } }));
    await page.evaluate(() => {
      window.__folderSelected = false;
      window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { folder: "No directory" } }));
    });
    await fire("nav-files");
    await control("save-99", { enabled: false });
    await fire("file-filter", { text: "missing", revision: 1 });
    await control("files-no-match", { label: "No matching files" });
    await fire("clear-file-filter");
    await control("save-99", { enabled: false });
    await fire("file-order");
    await fire("file-orderlargest");
    await control("file-order", { detail: "Size, largest first" });
    await fire("save-99");
    assert.ok(!await page.evaluate(() => window.__nativeCalls.some(([action]) => action === "download")));
    await fire("pick-folder");
    await control("save-99", { enabled: true });
    const longFolderPath = "Documents/Project Notes/项目资料/".repeat(18);
    await page.evaluate(folder => window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { folder } })), longFolderPath);
    await fire("folder");
    await control("folder-path", { label: longFolderPath });
    const previousFolderDismiss = await page.evaluate(() => window.__nativeUi.dismissAction);
    await fire("dismiss"); await fire("folder");
    await page.evaluate(id => window.dispatchEvent(new CustomEvent("plumelink-native-ui", { detail: { id } })), previousFolderDismiss);
    assert.equal(await page.evaluate(() => window.__nativeUi.sheetKey), "folder", "old folder dismiss cannot close its new opening");
    await page.evaluate(() => {
      window.__folderSelected = false;
      window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { folder: "No directory" } }));
    });
    await page.waitForFunction(() => !window.__nativeUi.sheetKey);
    await control("save-99", { enabled: false });
    await fire("pick-folder");
    await control("save-99", { enabled: true });
    await fire("save-99");
    await fire("cancel-transfer");
    await control("cancel-transfer", { enabled: false });
    await page.waitForFunction(() => !window.__nativeUi.controls.some(c => c.id === "transfer-card"), undefined, { timeout: 20000 });
    await control("upload", { enabled: false });
    await control("save-99", { enabled: false });
    const picksWhileStopping = await page.evaluate(() => window.__nativeCalls.filter(([name]) => name === "choose-upload").length);
    await fire("upload");
    assert.equal(await page.evaluate(() => window.__nativeCalls.filter(([name]) => name === "choose-upload").length), picksWhileStopping);
    await page.evaluate(() => {
      window.PlumeLinkAndroid.syncSession = () => window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
        detail: { progress: 0, done: true, succeeded: false, snapshot: true },
      }));
    });
    await fire("reload");
    await control("upload", { enabled: true });
    await control("save-99", { enabled: true });
    await fire("upload");
    await fire("save-99");
    await control("progress", { value: 0 });
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { progress: 42, done: false } })));
    await control("progress", { value: 42 });
    await fire("cancel-transfer");
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { progress: 0, done: true } })));
    await control("save-99", { enabled: true });
    for (const action of ["choose-upload", "download", "cancel-transfer"]) {
      assert.ok(await page.evaluate(action => window.__nativeCalls.some(([name]) => name === action), action));
    }
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
      detail: { done: true, succeeded: true, message: "已保存 test.bin", direction: "receive", fileName: "test.bin", fileCount: 1 },
    })));
    await fire("history-toggle");
    await control("history-0", { label: "Saved test.bin" });
    const unfinishedId = "b".repeat(32), unfinishedVersion = "c".repeat(64);
    const unfinishedAction = action => `unfinished-${action}-${unfinishedId}-${unfinishedVersion}`;
    await page.evaluate(({ id, version }) => window.dispatchEvent(new CustomEvent("plumelink-download-recovery", { detail: { files: [
      { id, version, name: "interrupted.bin", size: 512, expected: 1024, available: true, folder: "Downloads" },
    ], message: "" } })), { id: unfinishedId, version: unfinishedVersion });
    await fire("unfinished");
    await fire(unfinishedAction("delete"));
    await control(unfinishedAction("confirm-delete"), { label: "Confirm delete" });
    await fire(unfinishedAction("cancel-delete"));
    assert.equal(await page.evaluate(() => window.__nativeCalls.filter(([name]) => name === "resolve-download").length), 0, "a second tap in the same position cancels deletion");
    await fire(unfinishedAction("delete"));
    await fire(unfinishedAction("confirm-delete"));
    const removal = await page.evaluate(() => window.__nativeCalls.filter(([name]) => name === "resolve-download").at(-1));
    assert.deepEqual(removal.slice(0, 4), ["resolve-download", unfinishedId, unfinishedVersion, false]);
    assert.match(removal[4], /^[a-f0-9]{32}$/);
    await control(unfinishedAction("keep"), { enabled: false });
    await page.evaluate(request => window.dispatchEvent(new CustomEvent("plumelink-download-recovery", { detail: { files: [], message: "", request } })), removal[4]);
    await control("unfinished-empty", { label: "No unfinished files" });
    await fire("dismiss");
    await fire("nav-settings"); await fire("language"); await fire("languagezh-TW"); await fire("nav-files");
    await control("history-0", { label: "已保存 test.bin" });
    await fire("nav-settings"); await fire("language"); await fire("languageen"); await fire("nav-files");
    await control("history-0", { label: "Saved test.bin" });
    assert.ok(commands.some(c => c.type === "key" && c.key === "enter"));
    assert.ok(commands.some(c => c.type === "click" && c.button === "left"));
    assert.ok(commands.some(c => c.type === "volume" && c.action === "down"));
    assert.ok(commands.some(c => c.type === "key" && c.key === "space"));
    assert.ok(commands.some(c => c.type === "motion" && (c.dx || c.dy)));

    assert.ok(commands.some(c => c.type === "text" && c.text === "原生输入"));
    assert.deepEqual(errors, []);
    await fire("nav-settings"); await fire("control-mode"); await fire("control-modedpad"); await fire("nav-remote");
    await control("dpad");
    const beforeHold = commands.filter(c => c.type === "key" && c.key === "up").length;
    await fire("dpad-up", true);
    await expect.poll(() => commands.filter(c => c.type === "key" && c.key === "up").length).toBeGreaterThan(beforeHold + 2);
    await fire("dpad-up", false);
    await page.waitForTimeout(150);
    const stoppedHold = commands.filter(c => c.type === "key" && c.key === "up").length;
    await page.waitForTimeout(500);
    assert.equal(commands.filter(c => c.type === "key" && c.key === "up").length, stoppedHold);
    await page.evaluate(() => {
      const socket = window.__nativeControlSocket, receive = socket.onmessage;
      window.__delayedDpadAcks = 0;
      socket.onmessage = event => {
        if (event.data instanceof ArrayBuffer && event.data.byteLength === 64 && new Uint8Array(event.data)[4] === 6) {
          window.__delayedDpadAcks++;
          setTimeout(() => receive.call(socket, event), 300);
        } else receive.call(socket, event);
      };
      window.__restoreDpadAcks = () => { socket.onmessage = receive; };
    });
    const beforeDelayed = commands.filter(c => c.type === "key" && c.key === "right").length;
    await fire("dpad-right", true); await page.waitForTimeout(1500); await fire("dpad-right", false);
    await page.waitForTimeout(350);
    const delayedCount = commands.filter(c => c.type === "key" && c.key === "right").length;
    assert.ok(delayedCount - beforeDelayed >= 2 && delayedCount - beforeDelayed <= 5, "slow acknowledgements must not queue repeated keys");
    assert.ok(await page.evaluate(() => window.__delayedDpadAcks >= 2));
    await page.evaluate(() => window.__restoreDpadAcks());
    await page.waitForTimeout(500);
    assert.equal(commands.filter(c => c.type === "key" && c.key === "right").length, delayedCount);
    assert.equal(await page.evaluate(() => window.__nativeUi.feedback), "");
    const beforeCenter = commands.filter(c => c.type === "key" && c.key === "enter").length;
    await fire("dpad-center");
    await expect.poll(() => commands.filter(c => c.type === "key" && c.key === "enter").length).toBe(beforeCenter + 1);
    await fire("dpad-left", true); await fire("more");
    await page.waitForTimeout(150);
    const stoppedBySheet = commands.filter(c => c.type === "key" && c.key === "left").length;
    await page.waitForTimeout(600);
    assert.equal(commands.filter(c => c.type === "key" && c.key === "left").length, stoppedBySheet);
    await fire("dismiss");
    await fire("nav-settings"); await fire("control-mode"); await fire("control-modetrackpad"); await fire("nav-remote");
    assert.equal(await page.evaluate(() => window.__nativeUi.hardwareVolume), false);
    const volumeCount = action => commands.filter(c => c.type === "volume" && c.action === action).length;
    const beforeDisabledVolume = volumeCount("up");
    await fire("hardware-volume-up", true); await fire("hardware-volume-up", false);
    assert.equal(volumeCount("up"), beforeDisabledVolume);
    await fire("nav-settings"); await fire("hardware-volume", true); await fire("nav-remote");
    await page.waitForFunction(() => window.__nativeUi.hardwareVolume === true);
    const beforeVolumeHold = volumeCount("up");
    await fire("hardware-volume-up", true);
    await expect.poll(() => volumeCount("up")).toBeGreaterThanOrEqual(beforeVolumeHold + 3);
    await fire("hardware-volume-up", false); await page.waitForTimeout(150);
    const stoppedVolume = volumeCount("up"); await page.waitForTimeout(500);
    assert.equal(volumeCount("up"), stoppedVolume);
    await fire("hardware-volume-down", true); await fire("more");
    await page.waitForFunction(() => window.__nativeUi.hardwareVolume === false);
    await page.waitForTimeout(150); const menuVolume = volumeCount("down"); await page.waitForTimeout(500);
    assert.equal(volumeCount("down"), menuVolume);
    await fire("dismiss");
    await fire("hardware-volume-up", true); await fire("nav-files");
    await page.waitForFunction(() => window.__nativeUi.hardwareVolume === false);
    await page.waitForTimeout(150); const pageVolume = volumeCount("up"); await page.waitForTimeout(500);
    assert.equal(volumeCount("up"), pageVolume);
    await fire("nav-remote"); await fire("hardware-volume-down", true);
    const volumeSockets = frames.sockets;
    await page.evaluate(() => window.__nativeControlSocket.close());
    await expect.poll(() => frames.sockets).toBeGreaterThan(volumeSockets);
    await page.waitForFunction(() => window.__nativeUi.hardwareVolume === true);
    const reconnectVolume = volumeCount("down"); await page.waitForTimeout(600);
    assert.equal(volumeCount("down"), reconnectVolume, "a new connection must not resume held volume keys");
    await page.reload();
    await page.waitForFunction(() => window.__nativeUi?.sessionStatus === "connected" && window.__nativeUi.hardwareVolume === true);
    await fire("nav-settings"); await control("hardware-volume", { value: true });
    await fire("hardware-volume", false); await fire("nav-remote");
    await page.waitForFunction(() => window.__nativeUi.hardwareVolume === false);
    assert.deepEqual(errors, []);
    await writeFile(resolve(output, "native-ui-results.json"), JSON.stringify({
      passed: true, date: new Date().toISOString(),
      transport: "Android native bridge state -> Rust WASM -> WSS -> Rust Receiver -> fake Native",
      usbListener: usb, frames, commandCount: commands.length, nativeEffectsSimulated: true, errors,
    }, null, 2), { flag: requestedOutput ? "wx" : "w" });
    console.log("Native UI bridge actions passed using real QFT2/WSS.");
  } else if (mixed) {
    const { verifyConcurrency } = await import("./verify-concurrency.mjs");
    await verifyConcurrency({
      page,
      context,
      commands,
      frames,
      audioReports,
      fire, control, pointer,
      token: credentials.token,
    });
    assert.deepEqual(errors, []);
  }  else {
    await fire("left"); await fire("play"); await fire("keyboard");
    await control("draft");
    await fire("draft", { text: "QFT transmission test", revision: 1 });
    await fire("send");
    await expect.poll(() => commands.filter(c => c.type === "text").length).toBe(1);
    assert.equal(await page.evaluate(() => window.plumeLinkBack()), true);
    await page.waitForFunction(() => window.__nativeUi.destination === "remote");
    assert.ok(leaseMs > 0, "ready.qft.leaseMs must be available");
    const socketsBefore = frames.sockets, renewalWindow = leaseMs + 1200;
    await pointer("pad", "down", 90, 90);
    const until = Date.now() + renewalWindow;
    for (let n = 0; Date.now() < until; n++) {
      await pointer("pad", "move", 90 + (n % 30), 90 + (n % 20));
      await page.waitForTimeout(25);
    }
    await pointer("pad", "up", 100, 100);
    await page.waitForTimeout(renewalWindow * 2 + 100);
    await fire("right"); await page.waitForTimeout(100);
    assert.equal(frames.sockets, socketsBefore, "motion, releases and idle lease renewal must retain the same WSS");
    assert.ok(frames.received[1] >= 4 && frames.sent[2] > 30 && frames.sent[3] >= 4, JSON.stringify(frames));
    assert.ok(commands.some(c => c.type === "motion" && (c.dx || c.dy)));
    assert.ok(commands.some(c => c.type === "click" && c.button === "right"));
    await pointer("pad", "down", 90, 90);
    await expect.poll(() => commands.filter(c => c.type === "buttonstate" && c.down).length).toBeGreaterThan(0);
    const before = commands.filter(c => c.type === "buttonstate" && !c.down).length;
    await page.evaluate(() => window.dispatchEvent(new Event("pagehide")));
    await expect.poll(() => commands.filter(c => c.type === "buttonstate" && !c.down).length).toBeGreaterThan(before);
    assert.deepEqual(errors, []);
    await writeFile(resolve(output, "results.json"), JSON.stringify({ passed: true, date: new Date().toISOString(),
      transport: "Android native bridge state -> Rust WASM -> WSS -> Rust Receiver -> fake Native",
      frames, commandCount: commands.length, nativeEffectsSimulated: true, errors }, null, 2));
    console.log("QFT production integration passed:", JSON.stringify(frames));
  }

} catch (error) {
  await page
    ?.screenshot({ path: resolve(output, "failure.png") })
    .catch(() => {});
  const diagnostics = {
    frames,
    errors,
    trace,
    closures: await page
      ?.evaluate(() => window.__socketClosures ?? [])
      .catch(() => []),
    body: await page
      ?.locator("body")
      .innerText()
      .catch(() => "unavailable"),
  };
  await writeFile(
    resolve(output, `failure-${Date.now()}.json`),
    JSON.stringify(diagnostics, null, 2),
  );
  console.error("QFT integration diagnostics:", JSON.stringify(diagnostics));
  throw error;
} finally {
  await browser?.close();
  fixture.stdin.end("\n");
  await new Promise((resolveDone) => {
    const timeout = setTimeout(() => {
      fixture.kill();
      resolveDone();
    }, 4000);
    fixture.once("exit", () => {
      clearTimeout(timeout);
      resolveDone();
    });
  });
  lines.close();
}
