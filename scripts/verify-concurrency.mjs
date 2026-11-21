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

export async function verifyConcurrency({
  page,
  context,
  commands,
  frames,
  audioReports,
  token,
}) {
  const out = resolve("artifacts/concurrency");
  await mkdir(out, { recursive: true });
  const nav = (name) =>
    page
      .getByRole("navigation", { name: "主导航" })
      .getByRole("button", { name, exact: true })
      .click();
  const mic = page.getByRole("button", {
    name: "按住说话，上滑锁定",
    exact: true,
  });
  const r = await mic.boundingBox(),
    x = r.x + r.width / 2,
    y = r.y + r.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.waitForTimeout(300);
  await page.mouse.move(x, y - 85, { steps: 8 });
  await page.waitForTimeout(2200);
  await page.mouse.up();
  await page.locator(".mic-compact").waitFor();
  assert.ok(
    await page
      .locator(".mic-compact")
      .evaluate(
        (e) =>
          e.getBoundingClientRect().bottom <=
          document.querySelector(".touchpad").getBoundingClientRect().top,
      ),
  );
  await nav("文件");
  await page.getByText("concurrent-download.bin", { exact: true }).waitFor();
  const audioBefore = await page.evaluate(() => window.__audio.acks);
  const requests = [];
  page.on("request", (request) => {
    if (request.method() === "PUT" && request.url().includes("/api/uploads/"))
      requests.push(request.postDataBuffer().length);
  });
  await page.locator("input[type=file]").setInputFiles({
    name: "concurrent-upload.bin",
    mimeType: "application/octet-stream",
    buffer: Buffer.alloc(4 * 1024 ** 2, 0x33),
  });
  await page.evaluate((token) => {
    window.__download = { bytes: 0, done: false, error: null };
    void (async () => {
      try {
        const headers = { Authorization: `Bearer ${token}` },
          list = await (await fetch("/api/files", { headers })).json();
        const response = await fetch("/api/files/" + list.files[0].id, {
          headers,
        });
        if (!response.ok) throw Error("download rejected");
        const reader = response.body.getReader();
        while (true) {
          const { value, done } = await reader.read();
          if (done) break;
          if (value.some((v) => v !== 0x5a)) throw Error("download corrupted");
          window.__download.bytes += value.length;
        }
        window.__download.done = true;
      } catch (error) {
        window.__download.error = String(error);
      }
    })();
  }, token);
  await nav("遥控");
  const cdp = await context.newCDPSession(page),
    pad = await page.locator(".touchpad").boundingBox();
  const commandStart = commands.length;
  const began = Date.now();
  let n = 0;
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ id: 1, x: pad.x + 90, y: pad.y + 90 }],
  });
  const durationMs = Number(process.env.QFT_MIXED_DURATION_MS ?? 10000);
  assert.ok(
    Number.isFinite(durationMs) && durationMs >= 10000 && durationMs <= 300000,
  );
  while (Date.now() - began < durationMs) {
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ id: 1, x: pad.x + 90 + (n++ % 60), y: pad.y + 90 }],
    });
    await page.waitForTimeout(30);
  }
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchEnd",
    touchPoints: [],
  });
  assert.ok(
    commands.slice(commandStart).filter((v) => v.type === "motion").length > 80,
  );
  await page.waitForFunction(
    () => window.__download.done || window.__download.error,
    {},
    { timeout: 30000 },
  );
  const download = await page.evaluate(() => window.__download);
  assert.equal(download.error, null);
  assert.equal(download.bytes, 4 * 1024 ** 2);
  await nav("文件");
  await page
    .getByText("concurrent-upload.bin 已保存到电脑", { exact: true })
    .waitFor({ timeout: 30000 });
  assert.ok(
    requests.length >= 64 && requests.every((n) => n <= 64 * 1024),
    "live capture bounds upload bursts",
  );
  await nav("遥控");
  const prior = await page.evaluate(() => window.__audio.acks);
  await page.evaluate(() => window.__controlSocket.close());
  await page.getByRole("button", { name: /已连接/ }).waitFor();
  await page.waitForTimeout(1000);
  assert.ok(await page.locator(".mic-compact").isVisible());
  const metrics = await page.evaluate(() => window.__audio);
  assert.equal(metrics.connections, 1);
  assert.ok(metrics.acks > prior);
  assert.ok(metrics.acks - audioBefore > 300);
  assert.deepEqual(metrics.errors, []);
  const closures = await page.evaluate(() => window.__socketClosures);
  assert.equal(
    frames.sockets,
    3,
    "only audio, initial control and explicitly reconnected control: " +
      JSON.stringify(closures),
  );
  await page.screenshot({ path: resolve(out, "compact-microphone.png") });
  await page.locator(".mic-compact-stop").click();
  await page.waitForTimeout(300);
  assert.ok(audioReports.some((v) => v.samples > 48000));
  await page.getByRole("button", { name: "右键", exact: true }).click();
  await page.waitForTimeout(100);
  assert.ok(
    commands
      .slice(commandStart)
      .some((v) => v.type === "click" && v.button === "right"),
  );
  const rtts = metrics.rtts.sort((a, b) => a - b);
  const p95 = rtts[Math.floor(rtts.length * 0.95)];
  assert.ok(p95 < 200, `audio ack p95 ${p95} ms`);
  await writeFile(
    resolve(out, "results.json"),
    JSON.stringify(
      {
        passed: true,
        scope:
          "Production HTTPS/WSS, WASM input, streamed files, synthetic capture and audio renderer",
        uploadBytes: 4 * 1024 ** 2,
        download,
        uploadRequests: requests.length,
        audioFrames: metrics.acks,
        audioAckP95Ms: p95,
        audioAckMaxMs: rtts.at(-1),
        audioReports,
        frames,
        closures,
        inputDurationMs: durationMs,
        checks: [
          "simultaneous audio/input/upload/download",
          "navigation preserves locked microphone",
          "compact status stays outside touchpad",
          "control reconnect preserves audio",
          "audio stop preserves input",
        ],
      },
      null,
      2,
    ),
  );
  console.log("Concurrent transport checks passed; audio ACK p95:", p95);
}
