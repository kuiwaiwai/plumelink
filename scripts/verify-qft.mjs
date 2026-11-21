// Production mobile assets + real WASM/WSS/server/worker; only native effects are simulated.
import { spawn, spawnSync } from "node:child_process";
import { createInterface } from "node:readline";
import { readFile, mkdir, writeFile } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { expect } from "@playwright/test";

const build = spawnSync(
  process.execPath,
  ["scripts/cargo.mjs", "build", "--locked", "--example", "qft_fixture"],
  { stdio: "inherit", windowsHide: true },
);
if (build.status !== 0) throw new Error("QFT fixture build failed");
const root = resolve("tauri-dist/mobile"),
  output = resolve("artifacts/qft-integration");
await mkdir(output, { recursive: true });
const mixed = process.argv.includes("--mixed");
const nativeUi = process.argv.includes("--native-ui");
let leaseMs = 0;
const fixture = spawn(
  resolve(
    `src-tauri/target/debug/examples/qft_fixture${process.platform === "win32" ? ".exe" : ""}`,
  ),
  mixed ? ["--mixed"] : [],
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
    headers: { origin: ready.origin },
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
  if (process.argv.includes("--ui") || mixed || nativeUi)
    await context.addInitScript(useNativeUi => {
      window.__nativeCalls = [];
      window.__folderSelected = true;
      window.PlumeLinkAndroid = {
        scanPairingCode: () => window.__nativeCalls.push(['scan']),
        takePairingCode: () => { const code = sessionStorage.getItem('scanned-code') || ''; sessionStorage.removeItem('scanned-code'); return code; },
        getComputerName: () => "书房电脑",
        getDeviceName: () => "验收手机",
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
            if (window.__activeMic === id)
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
        changeComputer: () => window.__nativeCalls.push(["computers"]),
      };
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
    }, nativeUi);
  if (mixed) {
    const { installMixedAudio } = await import("./verify-concurrency.mjs");
    await context.addInitScript(installMixedAudio, credentials);
  }
  page = await context.newPage();
  // Mirrors Android's local asset interception; /api and /ws use the real server.
  await page.route(`${ready.origin}/**`, async (route) => {
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
  });
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
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
  await page.goto(ready.origin);
  if (!nativeUi) await page.getByRole("button", { name: /^已连接/ }).waitFor();
  if (nativeUi) {
    const fire = (id, value = null) => page.evaluate(({ id, value }) =>
      window.dispatchEvent(new CustomEvent("plumelink-native-ui", { detail: { id, value } })), { id, value });
    const control = (id, expected = {}) => page.waitForFunction(({ id, expected }) => {
      const find = items => {
        for (const item of items || []) {
          if (item.id === id && Object.entries(expected).every(([key, value]) => item[key] === value)) return true;
          if (find(item.children)) return true;
        }
        return false;
      };
      return window.__nativeUi && find([...window.__nativeUi.controls, ...window.__nativeUi.sheet]);
    }, { id, expected });
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
    const pointer = (target, phase, x, y) => page.evaluate(detail =>
      window.dispatchEvent(new CustomEvent("plumelink-native-pointer", { detail })),
    { target, phase, pointerId: 1, x, y, width: 280, primary: true });
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
    await fire("nav-files");
    await page.waitForFunction(() => window.__nativeUi.destination === "files");
    await micState({ expanded: false, recording: true, locked: true });
    await fire("mic-stop");
    await page.waitForFunction(() => window.__activeMic === null);
    await fire("nav-remote");
    await fire("more");
    await page.waitForFunction(() => window.__nativeUi.sheetKey === "more");
    const fixtureWindow = { hwnd: 42, title: "Editor", app: "editor.exe", minimized: true };
    await page.route("**/api/windows", route => route.fulfill({ json: { windows: [fixtureWindow] } }));
    let switchedWindow;
    await page.route("**/api/windows/switch", route => {
      switchedWindow = route.request().postDataJSON()?.hwnd;
      return route.fulfill({ json: { switched: true } });
    });
    await fire("windows");
    await page.waitForFunction(() => window.__nativeUi.destination === "windows");
    await control("window-42", { label: "Editor", detail: "editor.exe", status: "已最小化" });
    let reads = 0;
    let unavailable = true;
    await page.route("**/api/windows", route => {
      reads++;
      return unavailable ? route.fulfill({ status: 503, json: { error: "test offline" } })
        : route.fulfill({ json: { windows: [fixtureWindow] } });
    });
    async function expectHttpFailure(path, status, trigger) {
      const logged = page.waitForEvent("console", message =>
        message.type() === "error" && message.location().url === ready.origin + path &&
        message.text().includes("status of " + status + " ("));
      await trigger();
      const index = errors.indexOf((await logged).text());
      assert.ok(index >= 0);
      errors.splice(index, 1); // Only the deliberately injected response is expected.
    }
    await expectHttpFailure("/api/windows", 503, () => fire("window-refresh"));
    await control("windows-error");
    assert.ok(!await page.evaluate(() => window.__nativeUi.controls.some(c => c.id === "window-42")));
    unavailable = false;
    await page.evaluate(() => window.__nativeControlSocket.close());
    await control("windows-offline");
    await control("window-42", { enabled: true });
    assert.equal(reads, 2, "reconnection must fetch a fresh list");
    await fire("window-42");
    await control("windows-notice", { label: "已切换到 Editor" });
    assert.equal(switchedWindow, 42);
    await page.route("**/api/windows/switch", route => route.fulfill({ status: 409,
      json: { error: "窗口尚未切到前台，请打开任务视图选择窗口" } }));
    await expectHttpFailure("/api/windows/switch", 409, () => fire("window-42"));
    await control("windows-error", { label: "窗口尚未切到前台，请打开任务视图选择窗口" });
    await control("open-taskview", { enabled: true });
    const taskViewsBefore = commands.filter(c => c.type === "taskview").length;
    await fire("open-taskview");
    await page.waitForFunction(() => window.__nativeUi.destination === "remote");
    assert.equal(commands.filter(c => c.type === "taskview").length, taskViewsBefore + 1);
    await fire("keyboard");
    await page.waitForFunction(() => window.__nativeUi.destination === "keyboard");
    await fire("draft", { text: "原生输入", revision: 1 });
    await fire("send");
    await control("text-feedback");
    await fire("back-remote");
    await fire("nav-settings");
    await page.waitForFunction(() => window.__nativeUi.destination === "settings");
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
      await fire("nav-remote");
      await page.waitForFunction(title => window.__nativeUi.title === title, remote);
      await fire("more");
      await page.waitForFunction(title => window.__nativeUi.sheetTitle === title, more);
      await fire("windows");
      await page.waitForFunction(title => window.__nativeUi.destination === "windows" && window.__nativeUi.title === title, windows);
      await fire("back-remote");
      await fire("nav-files");
      await page.waitForFunction(title => window.__nativeUi.title === title, files);
      await control("receive", { label: receive });
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
    await fire("save-99");
    assert.ok(!await page.evaluate(() => window.__nativeCalls.some(([action]) => action === "download")));
    await fire("pick-folder");
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
    assert.ok(commands.some(c => c.type === "key" && c.key === "enter"));
    assert.ok(commands.some(c => c.type === "click" && c.button === "left"));
    assert.ok(commands.some(c => c.type === "volume" && c.action === "down"));
    assert.ok(commands.some(c => c.type === "key" && c.key === "space"));
    assert.ok(commands.some(c => c.type === "motion" && (c.dx || c.dy)));

    assert.ok(commands.some(c => c.type === "text" && c.text === "原生输入"));
    assert.deepEqual(errors, []);
    console.log("Native UI bridge actions passed using real QFT2/WSS.");
  } else if (mixed) {
    const { verifyConcurrency } = await import("./verify-concurrency.mjs");
    await verifyConcurrency({
      page,
      context,
      commands,
      frames,
      audioReports,
      token: credentials.token,
    });
    assert.deepEqual(errors, []);
  } else if (process.argv.includes("--ui")) {
    const { verifyMobileUI } = await import("./verify-mobile-ui.mjs");
    await verifyMobileUI({ page, context, commands, output, frames });
    assert.deepEqual(errors, []);
    console.log("Android app UI checks passed using real QFT2/WSS.");
  } else {
    await page.getByRole("button", { name: "设置", exact: true }).click();
    await page.getByRole("button", { name: /当前电脑/ }).click();
    await page.getByText("连接详情", { exact: true }).click();
    await page.getByText(/QFastTransmission ·/).scrollIntoViewIfNeeded();
    await page.screenshot({ path: resolve(output, "settings.png") });
    await page.keyboard.press("Escape");
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    await page.getByRole("button", { name: "遥控", exact: true }).click();
    await page.getByRole("button", { name: "左键", exact: true }).click();
    await page.getByRole("button", { name: "播放/暂停", exact: true }).click();
    await page.getByRole("button", { name: "键盘", exact: true }).click();
    await page.locator("#remote-text").fill("QFT 中文与 Unicode 🛰️");
    await page.getByRole("button", { name: "发送文字", exact: true }).click();
    await page.waitForFunction(
      () => document.querySelector("#remote-text").value === "",
    );
    await page.keyboard.press("Escape");
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    const cdp = await context.newCDPSession(page);
    const pad = await page.locator(".touchpad").boundingBox();
    const point = (n) => ({
      id: 1,
      x: pad.x + 90 + (n % 30),
      y: pad.y + 80 + (n % 20),
    });
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart",
      touchPoints: [point(0)],
    });
    // Cross the negotiated lease during motion, then observe two silent renewals.
    assert.ok(leaseMs > 0, "ready.qft.leaseMs must be available");
    const renewalWindow = leaseMs + 1200;
    const until = Date.now() + renewalWindow;
    for (let n = 1; Date.now() < until; n++) {
      await cdp.send("Input.dispatchTouchEvent", {
        type: "touchMove",
        touchPoints: [point(n)],
      });
      await page.waitForTimeout(25);
    }
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchEnd",
      touchPoints: [],
    });
    await page.waitForTimeout(renewalWindow * 2 + 100);
    await page.getByRole("button", { name: "右键", exact: true }).click();
    await page.waitForTimeout(100);
    assert.equal(
      frames.sockets,
      1,
      "normal releases and lease renewal must retain the same WSS",
    );
    assert.ok(frames.received[1] >= 4, JSON.stringify(frames));
    assert.ok(
      frames.sent[2] > 30 && frames.sent[3] >= 4,
      JSON.stringify(frames),
    );
    assert.equal(commands.filter((c) => c.type === "text").length, 1);
    assert.ok(commands.some((c) => c.type === "motion" && (c.dx || c.dy)));
    assert.ok(commands.some((c) => c.type === "click" && c.button === "right"));
    const before = commands.filter((c) => c.type === "release").length;
    await page.evaluate(() => window.dispatchEvent(new Event("pagehide")));
    await page.waitForTimeout(100);
    assert.ok(
      commands.filter((c) => c.type === "release").length > before,
      "backgrounding must release input",
    );
    assert.deepEqual(errors, []);
    await writeFile(
      resolve(output, "results.json"),
      JSON.stringify(
        {
          passed: true,
          date: new Date().toISOString(),
          transport:
            "Rust WASM → WSS → Rust Receiver → synchronous fake Native",
          frames,
          commandCount: commands.length,
          nativeEffectsSimulated: true,
          errors,
        },
        null,
        2,
      ),
    );
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
