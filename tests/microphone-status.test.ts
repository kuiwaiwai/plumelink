import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { effectScope, nextTick, ref, type EffectScope } from "vue";
import { languagePreference } from "../shared/i18n";
import { canStartMicrophone, canRestartMicrophone, microphoneGuidance, parseMicrophoneStatus, useMicrophoneStatus } from "../apps/mobile/composables/useMicrophoneStatus";

let scope: EffectScope;
const value = () => ({ configured: true, allowed: true, output: "CABLE Input", gainDb: 6, owned: true, busy: false,
  status: { active: true, error: "", receivedFrames: 8, receivedSignalFrames: 4, renderedSignalSamples: 80, queuedMs: 40, targetMs: 40, jitterMs: 2, underruns: 0, droppedFrames: 0 } });
const response = (data: unknown = value()) => new Response(JSON.stringify(data), { headers: { "Content-Type": "application/json" } });
const flush = async () => { await nextTick(); await vi.advanceTimersByTimeAsync(0); };
function deferred<T>() { let resolve!: (v: T) => void; const promise = new Promise<T>(yes => { resolve = yes; }); return { resolve, promise }; }
beforeEach(() => { scope = effectScope(); vi.useFakeTimers(); languagePreference.value = "zh-CN"; });
afterEach(() => { scope.stop(); vi.useRealTimers(); languagePreference.value = "system"; });
function setup(open = true, api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>().mockImplementation(async () => response())) {
  const shown = ref(open), online = ref(true), allowed = ref(true), foreground = ref(true);
  const model = scope.run(() => useMicrophoneStatus({ api }, shown, online, allowed, foreground))!;
  return { model, shown, online, allowed, foreground, api };
}
it("reads only while shown and does not overlap a pending response", async () => {
  const pending = deferred<Response>();
  const api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>().mockReturnValueOnce(pending.promise).mockImplementation(async () => response());
  const f = setup(false, api); await flush(); expect(api).not.toHaveBeenCalled();
  f.shown.value = true; await flush(); f.model.refresh();
  expect(api).toHaveBeenCalledTimes(1);
  pending.resolve(response()); await flush();
  await vi.advanceTimersByTimeAsync(1999); expect(api).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1); expect(api).toHaveBeenCalledTimes(2);
});
it.each(["shown", "online", "allowed", "foreground"] as const)("clears status and ignores late responses when %s stops", async name => {
  const pending = deferred<Response>(); const api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>().mockReturnValue(pending.promise);
  const f = setup(true, api); await flush();
  f[name].value = false; await flush();
  expect(api.mock.calls[0][1]?.signal?.aborted).toBe(true);
  pending.resolve(response()); await flush(); expect(f.model.data.value).toBeUndefined();
  await vi.advanceTimersByTimeAsync(10000); expect(api).toHaveBeenCalledTimes(1);
});
it("keeps an expired read from publishing a late success", async () => {
  const pending = deferred<Response>(); const api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>().mockReturnValue(pending.promise);
  const f = setup(true, api); await flush(); await vi.advanceTimersByTimeAsync(4000);
  expect(api.mock.calls[0][1]?.signal?.aborted).toBe(true);
  pending.resolve(response()); await flush();
  expect(f.model.data.value).toBeUndefined(); expect(f.model.error.value).toContain("超时");
});
it("stops retries for a non-JSON 404 until a deliberate refresh", async () => {
  const api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>().mockImplementation(async () => new Response("not found", { status: 404 }));
  const f = setup(true, api); await flush(); expect(f.model.unsupported.value).toBe(true);
  await vi.advanceTimersByTimeAsync(30000); expect(api).toHaveBeenCalledTimes(1);
  f.model.refresh(); await flush(); expect(api).toHaveBeenCalledTimes(2);
});
it.each([403, 200])("clears configuration and stops polling when the desktop denies access with HTTP %i", async status => {
  const api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>().mockImplementationOnce(async () => response())
    .mockImplementationOnce(async () => status === 403 ? new Response("forbidden", { status: 403 }) : response({ ...value(), allowed: false, output: "Private output", gainDb: 24 }))
    .mockImplementationOnce(async () => response());
  const f = setup(true, api); await flush(); expect(f.model.data.value?.output).toBe("CABLE Input");
  f.model.refresh(); await flush();
  expect(f.model.denied.value).toBe(true); expect(f.model.data.value).toBeUndefined(); expect(f.model.error.value).toBe("");
  await vi.advanceTimersByTimeAsync(20000); expect(api).toHaveBeenCalledTimes(2);
  f.model.refresh(); await flush(); expect(f.model.denied.value).toBe(false); expect(f.model.data.value?.output).toBe("CABLE Input");
});
it("retains a failed read warning during retries until fresh data is validated", async () => {
  const pending = deferred<Response>();
  const api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>().mockRejectedValueOnce(new Error("read failed")).mockReturnValueOnce(pending.promise);
  const f = setup(true, api); await flush(); expect(f.model.error.value).toBe("read failed");
  f.model.refresh(); await flush(); expect(f.model.error.value).toBe("read failed");
  pending.resolve(response()); await flush(); expect(f.model.error.value).toBe("");
});
it("discards old response ownership after reopening and disposes pending work", async () => {
  const old = deferred<Response>(), current = deferred<Response>();
  const api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>().mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
  const f = setup(true, api); await flush(); f.shown.value = false; await flush(); f.shown.value = true; await flush();
  old.resolve(response({ ...value(), output: "Old PC" })); await flush(); expect(f.model.data.value).toBeUndefined();
  current.resolve(response()); await flush(); expect(f.model.data.value?.output).toBe("CABLE Input");
  scope.stop(); await vi.advanceTimersByTimeAsync(10000); expect(api).toHaveBeenCalledTimes(2);
});
it("rejects invalid counters and strips unowned or legacy audio details", () => {
  for (const bad of [null, { ...value(), gainDb: 50 }, { ...value(), status: { ...value().status, receivedFrames: -1 } },
    { ...value(), status: { ...value().status, jitterMs: Infinity } }]) expect(parseMicrophoneStatus(bad)).toBeUndefined();
  for (const owned of [false, undefined]) {
    const parsed = parseMicrophoneStatus({ ...value(), owned, status: { ...value().status, error: "Other private error" } })!;
    expect(parsed.status.active).toBe(false); expect(parsed.status.error).toBe(""); expect(parsed.status.receivedFrames).toBe(0);
  }
  const denied = parseMicrophoneStatus({ ...value(), allowed: false, output: "Private output" })!;
  expect(denied.output).toBe(""); expect(denied.gainDb).toBeUndefined(); expect(denied.status.receivedFrames).toBe(0);
});
it("prioritizes access and setup guidance and treats silent audio as a valid stream", () => {
  const state = parseMicrophoneStatus(value())!;
  const help = (online = true, allowed = true, data = state) => microphoneGuidance(online, allowed, data, "live", "", "");
  expect(help(false).title).toBe("遥控连接已断开，麦克风仍在收音"); expect(help(true, false).title).toBe("尚未获得麦克风权限");
  expect(help(true, true, { ...state, configured: false }).title).toBe("电脑尚未配置音频设备");
  expect(microphoneGuidance(true, true, state, "idle", "", "", false).title).toBe("手机尚未允许录音");
  expect(help(true, true, { ...state, busy: true }).title).toBe("另一台手机正在使用麦克风通道");
  expect(help(true, true, { ...state, status: { ...state.status, receivedSignalFrames: 0 } }).title).toBe("已收到音频，暂未检测到声音");
  expect(help().title).toBe("声音已送往电脑音频设备"); expect(help().help).toContain("不代表软件已开始录音");
});

it("keeps automatic polling quiet and gives manual refresh shared, observable progress", async () => {
  const pending = deferred<Response>();
  const api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>()
    .mockImplementationOnce(async () => response()).mockReturnValueOnce(pending.promise);
  const f = setup(true, api); await flush();
  await vi.advanceTimersByTimeAsync(2000);
  expect(f.model.loading.value).toBe(false); expect(f.model.manualRefreshing.value).toBe(false);
  const first = f.model.refresh(), second = f.model.refresh();
  expect(first).toBe(second); expect(f.model.manualRefreshing.value).toBe(true); expect(api).toHaveBeenCalledTimes(2);
  pending.resolve(response()); await first;
  expect(f.model.manualRefreshing.value).toBe(false); expect(api).toHaveBeenCalledTimes(2);
});

it("does not let an old read clear the current manual refresh after reopening", async () => {
  const old = deferred<Response>(), current = deferred<Response>();
  const api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>()
    .mockImplementationOnce(async () => response()).mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
  const f = setup(true, api); await flush(); f.model.refresh();
  expect(f.model.manualRefreshing.value).toBe(true);
  f.shown.value = false; await flush(); expect(f.model.manualRefreshing.value).toBe(false);
  f.shown.value = true; await flush(); f.model.refresh();
  old.resolve(response()); await flush(); expect(f.model.manualRefreshing.value).toBe(true);
  current.resolve(response()); await flush(); expect(f.model.manualRefreshing.value).toBe(false);
  expect(api).toHaveBeenCalledTimes(3);
});

it("ends manual progress and retains a useful error when its network deadline expires", async () => {
  const api = vi.fn<(path: string, init?: RequestInit) => Promise<Response>>()
    .mockImplementationOnce(async () => response())
    .mockImplementationOnce((_path, init) => new Promise((_resolve, reject) => init!.signal!.addEventListener("abort", () => reject(new Error("aborted")), { once: true })));
  const f = setup(true, api); await flush(); f.model.refresh();
  await vi.advanceTimersByTimeAsync(4000);
  expect(f.model.manualRefreshing.value).toBe(false); expect(f.model.error.value).toContain("超时");
});

it.each([
  ["zh-CN", "遥控连接已断开，麦克风仍在收音", "正在连接音频通道", "尚未连接电脑"],
  ["zh-TW", "遙控連線已中斷，麥克風仍在收音", "正在連線音訊通道", "尚未連線電腦"],
  ["en", "Remote control disconnected; microphone audio is still active", "Connecting the audio channel", "PC not connected"],
] as const)("distinguishes independent audio and input connectivity in %s", (language, live, reconnecting, idle) => {
  languagePreference.value = language;
  const cached = parseMicrophoneStatus(value())!;
  expect(microphoneGuidance(false, true, cached, "live", "", "").title).toBe(live);
  expect(microphoneGuidance(false, true, cached, "reconnecting", "", "").title).toBe(reconnecting);
  expect(microphoneGuidance(false, true, cached, "idle", "", "").title).toBe(idle);
});

it("offers a new continuous session only after an eligible terminal failure", () => {
  const ready = { online: true, allowed: true, foreground: true, phoneAllowed: true, usbExpired: false, active: false,
    issue: "Previous failure", value: parseMicrophoneStatus({ ...value(), status: { ...value().status, active: false } })!, readError: "", reading: false };
  expect(canRestartMicrophone(ready)).toBe(true);
  for (const changed of [{ online: false }, { allowed: false }, { foreground: false }, { phoneAllowed: false }, { usbExpired: true },
    { active: true }, { issue: "" }, { value: undefined }, { readError: "Unable to read" }, { reading: true },
    { value: { ...ready.value, configured: false } }, { value: { ...ready.value, allowed: false } },
    { value: { ...ready.value, busy: true } }, { value: { ...ready.value, status: { ...ready.value.status, active: true } } }])
    expect(canRestartMicrophone({ ...ready, ...changed }), JSON.stringify(changed)).toBe(false);
});

it("offers an explicit first start only with a confirmed available route, without requiring prior phone permission", () => {
  const ready = { online: true, allowed: true, foreground: true, usbExpired: false, active: false, phoneAllowed: false,
    value: parseMicrophoneStatus({ ...value(), status: { ...value().status, active: false } })!, readError: "", reading: false };
  expect(canStartMicrophone(ready)).toBe(true);
  for (const changed of [{ online: false }, { allowed: false }, { foreground: false }, { usbExpired: true },
    { active: true }, { value: undefined }, { readError: "Unable to read" }, { reading: true },
    { value: { ...ready.value, configured: false } }, { value: { ...ready.value, allowed: false } },
    { value: { ...ready.value, busy: true } }, { value: { ...ready.value, status: { ...ready.value.status, active: true } } }])
    expect(canStartMicrophone({ ...ready, ...changed }), JSON.stringify(changed)).toBe(false);
});
