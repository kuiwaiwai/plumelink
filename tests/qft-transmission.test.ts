import { readFileSync } from "node:fs";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { languagePreference, t } from "../shared/i18n";
import {
  QftTransport,
  supportsQft,
  type Kernel,
  type QftCapability,
} from "../apps/mobile/qft-transport";

const capability: QftCapability = {
  version: 2,
  frameBytes: 64,
  session: "7",
  leaseMs: 5000,
  motionTtlMs: 120,
  actionTtlMs: 200,
  maxCatchup: 16384,
  adapter: "logical-pointer-v1",
};
const module = new WebAssembly.Module(
  readFileSync("apps/mobile/generated/qft.wasm"),
);
function frame(kind = 1, lease = 1, serial = 0, ttl = 5000) {
  const bytes = new ArrayBuffer(64),
    v = new DataView(bytes);
  v.setUint32(0, 0x51465432);
  v.setUint8(4, kind);
  v.setBigUint64(8, 7n, true);
  v.setUint32(16, lease, true);
  v.setUint32(48, serial, true);
  if (kind === 1) v.setUint16(42, ttl, true);
  return bytes;
}
function ack(sent: ArrayBuffer, status: number) {
  const source = new DataView(sent),
    bytes = frame(6, source.getUint32(16, true), source.getUint32(48, true)),
    v = new DataView(bytes);
  v.setUint32(20, source.getUint32(20, true), true);
  if (source.getUint8(4) === 3 || source.getUint8(4) === 7)
    v.setUint32(24, source.getUint32(24, true), true);
  v.setUint32(52, status, true);
  return bytes;
}
function setup(overrides: Partial<QftCapability> = {}) {
  const negotiated = { ...capability, ...overrides };
  const kernel = new WebAssembly.Instance(module, {})
    .exports as unknown as Kernel;
  const sent: (string | ArrayBuffer)[] = [];
  const socket = {
    bufferedAmount: 0,
    readyState: 1,
    send: (bytes: string | ArrayBufferLike | Blob | ArrayBufferView) => {
      if (typeof bytes !== "string" && !(bytes instanceof ArrayBuffer))
        throw new Error("unexpected data");
      sent.push(bytes);
    },
    close: vi.fn(),
  };
  const ready = vi.fn(),
    fault = vi.fn();
  const transport = new QftTransport(
    kernel,
    socket,
    negotiated,
    16000,
    ready,
    fault,
  );
  transport.start();
  transport.receive(frame(1, 1, 0, negotiated.leaseMs));
  return {
    transport,
    kernel,
    socket,
    sent,
    ready,
    fault,
    packets: () =>
      sent.filter((b): b is ArrayBuffer => b instanceof ArrayBuffer),
  };
}
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
});
afterEach(() => {
  languagePreference.value = "zh-CN";
  vi.useRealTimers();
});

function failureFor(packet: ArrayBuffer) {
  const frame = new DataView(packet);
  return { session: "7", lease: frame.getUint32(16, true), actionId: frame.getUint32(24, true),
    stage: frame.getInt32(44, true), message: "此应用需要管理员权限，请在电脑端启动" };
}
function confirmFailure(s: ReturnType<typeof setup>, packet: ArrayBuffer, status: number) {
  s.transport.receive(ack(packet, status));
  vi.advanceTimersByTime(16);
  const probe = s.packets().at(-1)!;
  expect(new DataView(probe).getUint8(4)).toBe(7);
  s.transport.receive(ack(probe, status));
}

it.each(["zh-CN", "zh-TW", "en"])("only enriches an unknown outcome after matching binary evidence (%s)", async (language) => {
  languagePreference.value = language;
  const s = setup();
  const settled = vi.fn();
  const result = s.transport.send({ type: "launch", id: "a".repeat(32) }).catch(error => error.message).then(value => { settled(value); return value; });
  const packet = s.packets()[0];
  s.transport.receiveFailure(failureFor(packet));
  await Promise.resolve();
  expect(settled).not.toHaveBeenCalled();
  confirmFailure(s, packet, 6);
  const message = await result;
  expect(message).toContain(language === "en" ? "Start it on your PC" : language === "zh-TW" ? "系統管理員權限" : "管理员权限");
  expect(message).toContain(language === "en" ? "not be resent" : language === "zh-TW" ? "不會自動重發" : "不会自动重发");
  expect(s.sent.filter(value => typeof value === "string" && JSON.parse(value).kind === "qftStage")).toHaveLength(1);
  s.transport.dispose();
});

it.each(["zh-CN", "zh-TW", "en"])("folder failures retain uncertainty and never resend (%s)", async language => {
  languagePreference.value = language;
  const s = setup();
  const result = s.transport.send({ type: "launch", id: "d".repeat(32) }).catch(error => error.message);
  const packet = s.packets()[0];
  const reason = "无法打开文件夹，请在电脑端检查访问权限";
  s.transport.receiveFailure({ ...failureFor(packet), message: reason });
  confirmFailure(s, packet, 6);
  expect(await result).toBe(t(reason) + "\n" + t("结果未确认，请检查电脑；不会自动重发"));
  expect(s.sent.filter(value => typeof value === "string" && JSON.parse(value).kind === "qftStage")).toHaveLength(1);
  s.transport.dispose();
});

it.each([
  { session: "8" }, { lease: 2 }, { stage: 2 }, { actionId: 2 },
  { actionId: "1" }, { stage: null }, { message: "private local path" },
])("ignores unrelated or unrecognized failure details %j", async (change) => {
  const s = setup();
  const result = s.transport.send({ type: "key", key: "enter" }).catch(error => error.message);
  const packet = s.packets()[0];
  s.transport.receiveFailure({ ...failureFor(packet), ...change });
  confirmFailure(s, packet, 6);
  expect(await result).toBe("操作结果未知，请确认电脑状态；不会自动重发");
  s.transport.dispose();
});

it.each([1, 2])("a failure note cannot override a core outcome %s", async (status) => {
  const s = setup();
  const result = s.transport.send({ type: "key", key: "enter" }).then(() => "committed", error => error.message);
  const packet = s.packets()[0];
  s.transport.receiveFailure(failureFor(packet));
  if (status === 1) s.transport.receive(ack(packet, status));
  else confirmFailure(s, packet, status);
  expect(await result).toBe(status === 1 ? "committed" : "操作未执行");
  s.transport.dispose();
});

it("does not carry failure notes into subsequent actions or a disconnect", async () => {
  const s = setup();
  const first = s.transport.send({ type: "key", key: "enter" });
  const packet = s.packets()[0];
  s.transport.receiveFailure(failureFor(packet));
  s.transport.receive(ack(packet, 1));
  await first;
  const next = s.transport.send({ type: "key", key: "enter" }).catch(error => error.message);
  vi.advanceTimersByTime(16);
  s.transport.receiveFailure(failureFor(packet));
  confirmFailure(s, s.packets().at(-1)!, 6);
  expect(await next).toBe("操作结果未知，请确认电脑状态；不会自动重发");
  s.transport.dispose();
  const disconnected = setup();
  const result = disconnected.transport.send({ type: "key", key: "enter" }).catch(error => error.message);
  disconnected.transport.receiveFailure(failureFor(disconnected.packets()[0]));
  disconnected.transport.dispose();
  expect(await result).toBe("连接中断，操作结果可能未知；不会自动重发");
});

it("retunes the live pacer while an action awaits confirmation", async () => {
  const s = setup();
  const done = s.transport.send({ type: "key", key: "enter" });
  const packet = s.packets().at(-1)!;
  s.transport.setIntervalUs(12000);
  expect(s.transport.snapshot().intervalMs).toBe(12);
  s.transport.receive(ack(packet, 1));
  await done;
  s.transport.setIntervalUs(16000);
  expect(s.transport.snapshot().intervalMs).toBe(16);
  expect(s.ready).toHaveBeenCalledOnce();
  expect(s.socket.close).not.toHaveBeenCalled();
});

it("negotiates only the exact QFT2 adapter contract", () => {
  expect(supportsQft(capability)).toBe(true);
  for (const c of [
    { ...capability, session: "0" },
    { ...capability, adapter: "raw-mouse" },
    { ...capability, leaseMs: 500 },
    { ...capability, leaseMs: 5000.5 },
    { ...capability, motionTtlMs: 120.5 },
    { ...capability, actionTtlMs: 200.5 },
  ])
    expect(supportsQft(c)).toBe(false);
});
it("honors negotiated lease and action TTL instead of adapter defaults", async () => {
  const s = setup({ leaseMs: 15000, motionTtlMs: 400, actionTtlMs: 800 });
  expect(s.ready).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(3000);
  expect(s.packets()).toHaveLength(0);
  s.socket.bufferedAmount = 9000;
  const settled = vi.fn();
  void s.transport
    .send({ type: "click", button: "left", count: 1 })
    .catch((error) => error.message)
    .then(settled);
  await vi.advanceTimersByTimeAsync(260);
  expect(settled).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(600);
  expect(settled).toHaveBeenCalledWith(expect.stringContaining("等待过久"));
  s.transport.dispose();
});
it("runs the real Rust/WASM sender, freezes action prefixes, and confirms once", async () => {
  const s = setup();
  expect(s.ready).toHaveBeenCalledOnce();
  s.transport.motion({ dx: 3, dy: 4, scroll: 0 });
  const first = s.packets()[0];
  expect(new DataView(first).getUint8(4)).toBe(2);
  s.transport.motion({ dx: 2, dy: -1, scroll: 120 });
  const click = s.transport.send({ type: "click", button: "left", count: 1 });
  expect(
    s.sent
      .filter((b) => typeof b === "string")
      .map((b) => JSON.parse(b as string).kind),
  ).toEqual(["qftOpen", "qftStage"]);
  vi.advanceTimersByTime(16);
  const action = s.packets().at(-1)!,
    v = new DataView(action);
  expect([
    v.getUint8(4),
    v.getInt32(28, true),
    v.getInt32(32, true),
    v.getInt32(36, true),
  ]).toEqual([3, 5, 3, 120]);
  s.transport.motion({ dx: 1, dy: 0, scroll: 0 });
  s.transport.receive(ack(action, 1));
  await click;
  s.transport.receive(ack(action, 1));
  vi.advanceTimersByTime(16);
  expect(new DataView(s.packets().at(-1)!).getInt32(28, true)).toBe(6);
  s.transport.dispose();
});
it("retains the original action identity on retry and rejects unknown wire evidence", async () => {
  const s = setup();
  const done = s.transport
    .send({ type: "key", key: "enter" })
    .catch((e) => e.message);
  const first = s.packets()[0];
  vi.advanceTimersByTime(100);
  const retry = s.packets().at(-1)!;
  expect(new DataView(retry).getUint32(24, true)).toBe(
    new DataView(first).getUint32(24, true),
  );
  expect(new DataView(retry).getUint32(48, true)).toBeGreaterThan(
    new DataView(first).getUint32(48, true),
  );
  const bad = ack(first, 1);
  new DataView(bad).setBigUint64(8, 8n, true);
  s.transport.receive(bad);
  expect(s.socket.close).toHaveBeenCalledOnce();
  expect(await done).toContain("结果可能未知");
});

it("fences buffered drag movement with release after the press commits", async () => {
  const s = setup();
  const press = s.transport.send({ type: "buttonstate", button: "left", down: true });
  const first = s.packets()[0];
  s.transport.motion({ dx: 7, dy: -2, scroll: 0 });
  const release = s.transport.send({ type: "buttonstate", button: "left", down: false });
  expect(s.packets()).toHaveLength(1);
  s.transport.receive(ack(first, 1));
  await press;
  vi.advanceTimersByTime(18);
  const second = s.packets().at(-1)!;
  const view = new DataView(second);
  expect([view.getUint8(4), view.getInt32(28, true), view.getInt32(32, true)]).toEqual([3, 7, -2]);
  s.transport.receive(ack(second, 1));
  await release;
  s.transport.dispose();
});

it("expires a queued motion prefix without staging its dependent click", async () => {
  const s = setup();
  const first = s.transport.send({ type: "key", key: "enter" }).catch(() => {});
  const sent = s.packets()[0];
  s.transport.motion({ dx: 12, dy: 0, scroll: 0 });
  const click = s.transport.send({ type: "click", button: "left", count: 1 }).catch(e => e.message);
  await vi.advanceTimersByTimeAsync(121);
  s.transport.receive(ack(sent, 1));
  await first;
  expect(await click).toContain("等待过久");
  expect(s.sent.filter(packet => typeof packet === "string" && JSON.parse(packet).kind === "qftStage")).toHaveLength(1);
  s.transport.dispose();
});

it("rejects queued followers of a locally refused text step", async () => {
  const s = setup();
  for (let i = 0; i < 4; i++) {
    const typing = s.transport.send({ type: "text", text: String(i) });
    vi.advanceTimersByTime(16);
    s.transport.receive(ack(s.packets().at(-1)!, 1));
    await typing;
  }
  const prior = s.transport.send({ type: "key", key: "enter" });
  vi.advanceTimersByTime(16);
  const packet = s.packets().at(-1)!;
  s.transport.motion({ dx: 3, dy: 0, scroll: 0 });
  const text = s.transport.send({ type: "text", text: "fifth" }).catch(e => e.message);
  const click = s.transport.send({ type: "click", button: "left", count: 1 }).catch(e => e.message);
  s.transport.receive(ack(packet, 1));
  await prior;
  expect(await text).toContain("过于频繁");
  expect(await click).toContain("已取消");
  expect(s.sent.filter(p => typeof p === "string" && JSON.parse(p).kind === "qftStage")).toHaveLength(5);
  s.transport.dispose();
});
it("stops sending when the WebSocket starts closing before its close event", async () => {
  const s = setup();
  const first = s.transport.send({ type: "key", key: "enter" }).catch((error) => error.message);
  const queued = s.transport.send({ type: "key", key: "escape" }).catch((error) => error.message);
  expect(s.packets()).toHaveLength(1);
  const sentCount = s.sent.length;
  s.socket.readyState = WebSocket.CLOSING;
  await vi.advanceTimersByTimeAsync(200);
  expect(s.sent).toHaveLength(sentCount);
  expect(await first).toContain("结果可能未知");
  expect(await queued).toContain("操作未发送");
  expect(s.socket.close).not.toHaveBeenCalled();
});
it("honors an acknowledgment already received while the WebSocket is closing", async () => {
  const s = setup();
  const sent = s.transport.send({ type: "key", key: "enter" });
  const packet = s.packets()[0];
  const queued = s.transport.send({ type: "key", key: "escape" }).catch((error) => error.message);
  s.socket.readyState = WebSocket.CLOSING;
  s.transport.receive(ack(packet, 1));
  await expect(sent).resolves.toBeUndefined();
  expect(await queued).toContain("操作未发送");
  expect(s.packets()).toHaveLength(1);
});
it("renews during silence and accepts a delayed original grant after retry retirement", () => {
  const s = setup();
  vi.advanceTimersByTime(3000);
  const renew = s.packets().at(-1)!;
  expect(new DataView(renew).getUint8(4)).toBe(5);
  vi.advanceTimersByTime(140);
  s.transport.receive(frame(1, 2, new DataView(renew).getUint32(48, true)));
  expect(s.kernel.qft_metric(0)).toBe(8_000_000);
  expect(s.socket.close).not.toHaveBeenCalled();
  s.transport.dispose();
});
it("backpressure expires queued effects without silently refreshing their capture age", async () => {
  const s = setup();
  s.socket.bufferedAmount = 9000;
  const done = s.transport
    .send({ type: "click", button: "left", count: 1 })
    .catch((e) => e.message);
  s.transport.motion({ dx: 1, dy: 0, scroll: 0 });
  vi.advanceTimersByTime(260);
  expect(await done).toContain("等待过久");
  expect(s.packets()).toHaveLength(0);
  expect(s.socket.close).toHaveBeenCalled();
  s.transport.dispose();
});
it("discards queued input from the prior coordinate epoch without reconnecting after renewal", async () => {
  const s = setup();
  vi.advanceTimersByTime(3000);
  const renew = s.packets().at(-1)!;
  vi.advanceTimersByTime(5);
  s.transport.motion({ dx: 9, dy: 0, scroll: 0 });
  const click = s.transport
    .send({ type: "click", button: "left", count: 1 })
    .catch((e) => e.message);
  vi.advanceTimersByTime(5);
  s.transport.receive(frame(1, 2, new DataView(renew).getUint32(48, true)));
  expect(s.socket.close).not.toHaveBeenCalled();
  expect(await click).toContain("待发送操作已取消");
  expect(s.sent.filter((b) => typeof b === "string")).toHaveLength(1);
  s.transport.motion({ dx: 2, dy: 0, scroll: 0 });
  vi.advanceTimersByTime(16);
  const motion = new DataView(s.packets().at(-1)!);
  expect([motion.getUint32(16, true), motion.getInt32(28, true)]).toEqual([
    2, 2,
  ]);
  expect(s.kernel.qft_closed()).toBe(0);
  s.transport.dispose();
});
it.each([{ type: "release" } as const, { type: "buttonstate", button: "left", down: false } as const])(
  "preserves $type cleanup across renewal without replaying prior motion", async (cleanup) => {
  const s = setup();
  vi.advanceTimersByTime(3000);
  const renew = s.packets().at(-1)!;
  vi.advanceTimersByTime(5);
  s.transport.motion({ dx: 9, dy: 0, scroll: 0 });
  const release = s.transport.send(cleanup);
  vi.advanceTimersByTime(5);
  s.transport.receive(frame(1, 2, new DataView(renew).getUint32(48, true)));
  vi.advanceTimersByTime(16);
  const action = s.packets().at(-1)!;
  const view = new DataView(action);
  expect([
    view.getUint8(4),
    view.getUint32(16, true),
    view.getInt32(28, true),
  ]).toEqual([3, 2, 0]);
  expect(
    JSON.parse(s.sent.filter((b) => typeof b === "string").at(-1)! as string)
      .command,
  ).toEqual({ type: "release" });
  s.transport.receive(ack(action, 1));
  await release;
  expect(s.socket.close).not.toHaveBeenCalled();
  s.transport.dispose();
});
it("a release remains in the ordered action channel and does not reconnect a healthy session", async () => {
  const s = setup();
  const release = s.transport.send({ type: "release" });
  const action = s.packets()[0];
  s.transport.receive(ack(action, 1));
  await release;
  expect(s.kernel.qft_closed()).toBe(0);
  expect(s.socket.close).not.toHaveBeenCalled();
  s.transport.dispose();
});
it("cancellation can settle an already transmitted action without sending it again", async () => {
  const s = setup();
  const done = s.transport.send({ type: "click", button: "left", count: 1 });
  s.transport.cancel();
  const query = s.packets().at(-1)!;
  expect(new DataView(query).getUint8(4)).toBe(7);
  s.transport.receive(ack(query, 1));
  await done;
  vi.advanceTimersByTime(25);
  expect(s.socket.close).toHaveBeenCalled();
});
it("blocks a later click after a buffered pointer delta expired on its own", async () => {
  const s = setup();
  const click = s.transport.send({ type: "click", button: "left", count: 1 });
  const action = s.packets()[0];
  s.transport.motion({ dx: 12, dy: -6, scroll: 0 });
  vi.advanceTimersByTime(130);
  const lateClick = s.transport.send({ type: "click", button: "left", count: 1 });
  await expect(lateClick).rejects.toThrow();
  s.transport.receive(ack(action, 1));
  await click;
  expect(s.sent.filter(p => typeof p === "string" && JSON.parse(p).kind === "qftStage")).toHaveLength(1);
  expect(s.kernel.qft_closed()).toBe(1);
  s.transport.dispose();
});

it("reconciles delayed desktop text input without staging or executing it again", async () => {
  const s = setup();
  const sent = s.transport.send({ type: "text", text: "第一行\n第二行" });
  const original = s.packets()[0];
  vi.advanceTimersByTime(400);
  const afterCutoff = s.packets().length;
  vi.advanceTimersByTime(1400);
  expect(s.socket.close).not.toHaveBeenCalled();
  expect(s.packets().slice(afterCutoff).every(p => new DataView(p).getUint8(4) === 7)).toBe(true);
  expect(s.sent.filter(p => typeof p === "string" && JSON.parse(p).kind === "qftStage")).toHaveLength(1);
  s.transport.receive(ack(original, 1));
  await expect(sent).resolves.toBeUndefined();
  s.transport.dispose();
});

it("reports uncertainty when the bounded text confirmation window ends without evidence", async () => {
  const s = setup();
  const result = s.transport.send({ type: "text", text: "test" }).catch(e => e.message);
  vi.advanceTimersByTime(8500);
  expect(await result).toContain("操作结果未知");
  expect(s.socket.close).toHaveBeenCalledOnce();
});

it("keeps the connection usable when repeated text reaches the desktop submission limit", async () => {
  const s = setup({ leaseMs: 60000 });
  const submit = () => s.transport.send({ type: "text", text: "again" });
  for (let i = 0; i < 4; i++) {
    const done = submit();
    vi.advanceTimersByTime(16);
    s.transport.receive(ack(s.packets().at(-1)!, 1));
    await done;
  }
  const count = s.sent.length;
  await expect(submit()).rejects.toThrow("文字发送过于频繁");
  expect(s.sent).toHaveLength(count);
  vi.advanceTimersByTime(10000);
  const next = submit();
  vi.advanceTimersByTime(16);
  s.transport.receive(ack(s.packets().at(-1)!, 1));
  await expect(next).resolves.toBeUndefined();
  expect(s.socket.close).not.toHaveBeenCalled();
  s.transport.dispose();
});
