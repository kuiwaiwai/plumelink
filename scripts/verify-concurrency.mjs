import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

// Synthetic capture uses the real audio wire, WSS, authorization and server queues.
export function installMixedAudio({ token }) {
  const Base = window.WebSocket;
  window.__socketClosures = [];
  window.WebSocket = class extends Base {
    constructor(...args) {
      super(...args);
      this.addEventListener("close", (event) =>
        window.__socketClosures.push({
          url: String(args[0]),
          code: event.code,
          reason: event.reason,
          at: performance.now(),
        }),
      );
      if (String(args[0]).endsWith("/ws")) window.__controlSocket = this;
    }
  };
  window.__audio = {
    connections: 0,
    acks: 0,
    rtts: [],
    errors: [],
    stopped: 0,
  };
  let current;
  window.PlumeLinkAndroid.startMicrophone = (id) => {
    const metrics = window.__audio,
      ws = new WebSocket(
        location.origin.replace("https:", "wss:") + "/microphone",
      );
    metrics.connections++;
    const began = performance.now(),
      sent = new Map();
    let sequence = 0,
      timer;
    const state = (phase, message = "") =>
      window.dispatchEvent(
        new CustomEvent("plumelink-microphone", {
          detail: { id, phase, level: 0.6, message },
        }),
      );
    const session = {
      id,
      stop: () => {
        clearInterval(timer);
        ws.close();
      },
    };
    current = session;
    ws.onopen = () => ws.send(JSON.stringify({ kind: "auth", token }));
    ws.onmessage = (event) => {
      const data = JSON.parse(event.data);
      if (data.kind === "ready") {
        state("live");
        timer = setInterval(() => {
          if (ws.readyState !== 1 || sent.size >= 4) return;
          const frame = new ArrayBuffer(972),
            view = new DataView(frame);
          view.setUint32(0, 0x51464d31);
          view.setUint32(4, ++sequence, true);
          view.setUint32(8, Math.floor(performance.now() - began), true);
          for (let offset = 12; offset < 972; offset += 2)
            view.setInt16(offset, 1234, true);
          sent.set(sequence, performance.now());
          ws.send(frame);
        }, 20);
      } else if (data.kind === "ack") {
        metrics.acks++;
        metrics.rtts.push(performance.now() - sent.get(data.sequence));
        sent.delete(data.sequence);
      } else {
        metrics.errors.push(data);
        state("error", "音频测试连接失败");
      }
    };
    ws.onclose = () => {
      clearInterval(timer);
      if (current === session) {
        current = null;
        state("idle");
      }
    };
    ws.onerror = () => metrics.errors.push("WebSocket error");
  };
  window.PlumeLinkAndroid.stopMicrophone = (id) => {
    if (current?.id === id) {
      current.stop();
      current = null;
      window.__audio.stopped++;
    }
  };
}

// Models the native uploader for server scheduling checks; Android tests exercise its implementation.
export function installMixedUpload({ token }) {
  window.PlumeLinkAndroid.chooseUploadFiles = () => {
    window.__upload = { done: false, error: null, bytes: 0 };
    const name = "concurrent-upload.bin", size = 4 * 1024 ** 2;
    const progress = (done = false) => window.dispatchEvent(new CustomEvent("plumelink-native-transfer", {
      detail: { done, succeeded: done, direction: "send", fileName: name, fileCount: done ? 1 : 0,
        progress: Math.floor(window.__upload.bytes * 100 / size), message: done ? "Upload complete" : "Uploading" },
    }));
    progress();
    void (async () => {
      const id = crypto.randomUUID().replaceAll("-", ""), path = "/api/uploads/" + id;
      const request = async (path, init) => {
        const response = await fetch(path, { ...init, headers: { Authorization: `Bearer ${token}`, ...init.headers } });
        const data = await response.json();
        if (!response.ok) throw Error(data.error || "upload rejected");
        return data;
      };
      try {
        await request("/api/uploads", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id, name, size }) });
        while (window.__upload.bytes < size) {
          const bytes = new Uint8Array(64 * 1024).fill(0x33);
          const digest = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map(v => v.toString(16).padStart(2, "0")).join("");
          const state = await request(path, { method: "PUT", headers: { "Content-Type": "application/octet-stream", "X-Upload-Offset": String(window.__upload.bytes), "X-Chunk-SHA256": digest }, body: bytes });
          if (state.offset !== window.__upload.bytes + bytes.length) throw Error("invalid upload offset");
          window.__upload.bytes = state.offset; progress();
          await new Promise(resolve => setTimeout(resolve, 64));
        }
        const state = await request(path, { method: "POST" });
        if (!state.complete || state.offset !== size) throw Error("upload not committed");
        window.__upload.done = true; progress(true);
      } catch (error) {
        window.__upload.error = String(error);
        window.dispatchEvent(new CustomEvent("plumelink-native-transfer", { detail: { done: true, succeeded: false, message: String(error) } }));
      }
    })();
  };
}

export async function verifyConcurrency({ page, commands, frames, audioReports, token, fire, control, pointer }) {
  const out = resolve("artifacts/concurrency");
  await mkdir(out, { recursive: true });
  await pointer("mic", "down", 100, 300);
  await page.waitForTimeout(300);
  await pointer("mic", "move", 100, 100);
  await page.waitForTimeout(1200);
  await pointer("mic", "up", 100, 100);
  await page.waitForFunction(() => window.__nativeUi.microphone.locked);
  await fire("nav-files");
  await control("save-1");
  const audioBefore = await page.evaluate(() => window.__audio.acks);
  const requests = [];
  page.on("request", request => {
    if (request.method() === "PUT" && request.url().includes("/api/uploads/")) requests.push(request.postDataBuffer().length);
  });
  await fire("upload");
  await page.evaluate(token => {
    window.__download = { bytes: 0, done: false, error: null };
    void (async () => {
      try {
        const headers = { Authorization: `Bearer ${token}` };
        const list = await (await fetch("/api/files", { headers })).json();
        const response = await fetch("/api/files/" + list.files[0].id, { headers });
        if (!response.ok) throw Error("download rejected");
        const reader = response.body.getReader();
        while (true) {
          const { value, done } = await reader.read();
          if (done) break;
          if (value.some(v => v !== 0x5a)) throw Error("download corrupted");
          window.__download.bytes += value.length;
        }
        window.__download.done = true;
      } catch (error) { window.__download.error = String(error); }
    })();
  }, token);
  await fire("nav-remote");
  const commandStart = commands.length;
  const began = Date.now();
  let n = 0;
  await pointer("pad", "down", 90, 90);
  const durationMs = Number(process.env.QFT_MIXED_DURATION_MS ?? 10000);
  assert.ok(Number.isFinite(durationMs) && durationMs >= 10000 && durationMs <= 300000);
  while (Date.now() - began < durationMs) {
    await pointer("pad", "move", 90 + (n++ % 60), 90);
    await page.waitForTimeout(30);
  }
  await pointer("pad", "up", 90, 90);
  assert.ok(commands.slice(commandStart).filter(v => v.type === "motion").length > 80);
  await page.waitForFunction(() => window.__download.done || window.__download.error, {}, { timeout: 30000 });
  const download = await page.evaluate(() => window.__download);
  assert.equal(download.error, null);
  assert.equal(download.bytes, 4 * 1024 ** 2);
  await page.waitForFunction(() => window.__upload.done || window.__upload.error, {}, { timeout: 30000 });
  const upload = await page.evaluate(() => window.__upload);
  assert.equal(upload.error, null);
  assert.equal(upload.bytes, 4 * 1024 ** 2);
  assert.ok(requests.length >= 64 && requests.every(n => n <= 64 * 1024));
  const prior = await page.evaluate(() => window.__audio.acks);
  await page.evaluate(() => window.__controlSocket.close());
  await page.waitForFunction(() => window.__nativeUi.sessionStatus === "connected");
  await page.waitForTimeout(1000);
  assert.ok(await page.evaluate(() => window.__nativeUi.microphone.locked));
  const metrics = await page.evaluate(() => window.__audio);
  assert.equal(metrics.connections, 1);
  assert.ok(metrics.acks > prior && metrics.acks - audioBefore > 300);
  assert.deepEqual(metrics.errors, []);
  const closures = await page.evaluate(() => window.__socketClosures);
  assert.equal(frames.sockets, 3, JSON.stringify(closures));
  await fire("mic-stop");
  await page.waitForTimeout(300);
  assert.ok(audioReports.some(v => v.samples > 48000));
  await fire("right");
  await page.waitForTimeout(100);
  assert.ok(commands.slice(commandStart).some(v => v.type === "click" && v.button === "right"));
  const rtts = metrics.rtts.sort((a, b) => a - b), p95 = rtts[Math.floor(rtts.length * .95)];
  assert.ok(p95 < 200, `audio ack p95 ${p95} ms`);
  await writeFile(resolve(out, "results.json"), JSON.stringify({ passed: true,
    scope: "Production native-state engine, HTTPS/WSS and WASM input; synthetic native upload, capture and renderer",
    upload, download, uploadRequests: requests.length, audioFrames: metrics.acks, audioAckP95Ms: p95,
    audioAckMaxMs: rtts.at(-1), audioReports, frames, closures, inputDurationMs: durationMs,
    checks: ["simultaneous audio/input/upload/download", "navigation preserves locked microphone", "control reconnect preserves audio", "audio stop preserves input"] }, null, 2));
  console.log("Concurrent transport checks passed; audio ACK p95:", p95);
}
