import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { effectScope, ref, type EffectScope } from "vue";
import { useConnectionRecovery } from "../apps/mobile/composables/useConnectionRecovery";
import type { AndroidBridge } from "../apps/mobile/native/bridge";
import { languagePreference } from "../shared/i18n";

let scope: EffectScope, events: EventTarget;
beforeEach(() => {
  vi.useFakeTimers(); scope = effectScope(); events = new EventTarget();
  vi.stubGlobal("window", events); vi.stubGlobal("location", { hostname: "192.168.1.2" });
  languagePreference.value = "zh-CN";
});
afterEach(() => { scope.stop(); vi.useRealTimers(); vi.unstubAllGlobals(); languagePreference.value = "system"; });
function setup(restored = "") {
  const native = { recoverConnection: vi.fn(), cancelConnectionRecovery: vi.fn(), takeRecoveredDraft: vi.fn(() => restored) };
  const status = ref("offline"), draft = ref(""), open = ref(true), foreground = ref(true);
  const model = scope.run(() => useConnectionRecovery(native as unknown as AndroidBridge,
    { status, token: () => "a".repeat(64) }, draft, open, foreground))!;
  const event = (request: string, phase: string, message: unknown = "") => events.dispatchEvent(new CustomEvent("plumelink-connection-recovery", { detail: { request, phase, message } }));
  const request = () => native.recoverConnection.mock.calls.at(-1)![1] as string;
  return { native, status, draft, open, foreground, model, event, request };
}

it("only starts on an explicit foreground request and restores a draft without sending it", () => {
  const { native, draft, model, request } = setup("选词完成\nsecond line");
  expect(draft.value).toBe("选词完成\nsecond line"); expect(native.recoverConnection).not.toHaveBeenCalled();
  model.recover(); model.recover();
  expect(native.recoverConnection).toHaveBeenCalledOnce();
  expect(native.recoverConnection).toHaveBeenCalledWith(draft.value, expect.stringMatching(/^[a-f0-9]{32}$/));
  expect(model.busy.value).toBe(true); expect(request()).toHaveLength(32);
});
it("accepts only this request's terminal events and permits a failed attempt to be retried", () => {
  const { native, model, event, request } = setup(); model.recover(); const first = request();
  event("f".repeat(32), "error", "stale"); event(first, "unknown", "untrusted");
  expect(model.busy.value).toBe(true); expect(model.error.value).toBe("");
  event(first, "error", "原证书不一致"); expect(model.busy.value).toBe(false); expect(model.error.value).toBe("原证书不一致");
  model.recover(); const second = request(); expect(second).not.toBe(first);
  event(first, "cancelled"); expect(model.busy.value).toBe(true);
  event(second, "cancelled"); expect(model.busy.value).toBe(false); expect(native.cancelConnectionRecovery).not.toHaveBeenCalled();
});
it("retires the exact timed-out request and ignores its late error", async () => {
  const { native, draft, model, event, request } = setup("keep"); model.recover(); const first = request();
  await vi.advanceTimersByTimeAsync(15000);
  expect(model.busy.value).toBe(false); expect(native.cancelConnectionRecovery).toHaveBeenCalledExactlyOnceWith(first);
  expect(model.error.value).toContain("同一网络"); expect(draft.value).toBe("keep");
  model.recover(); event(first, "error", "late"); expect(model.busy.value).toBe(true); expect(model.error.value).toBe("");
});
it.each(["connected", "unpaired", "closed", "background"])("cancels when the original connection or lifecycle changes: %s", state => {
  const { native, status, open, foreground, model, request, event } = setup(); model.recover(); const id = request();
  if (state === "closed") open.value = false;
  else if (state === "background") foreground.value = false;
  else status.value = state;
  expect(native.cancelConnectionRecovery).toHaveBeenCalledExactlyOnceWith(id); expect(model.busy.value).toBe(false);
  event(id, "error", "late"); expect(model.error.value).toBe("");
  model.recover(); expect(native.recoverConnection).toHaveBeenCalledOnce();
});
it("does not recover USB or retain invalid draft data", () => {
  vi.stubGlobal("location", { hostname: "127.0.0.1" });
  const { native, draft, model } = setup("private\u0000invalid");
  expect(model.available.value).toBe(false); expect(draft.value).toBe(""); model.recover();
  expect(native.recoverConnection).not.toHaveBeenCalled();
});
it("releases a synchronous bridge failure and never truncates or submits an oversized draft", () => {
  const { native, draft, model } = setup("x".repeat(1001)); expect(draft.value).toBe("");
  draft.value = "x".repeat(1001); model.recover(); expect(native.recoverConnection).not.toHaveBeenCalled();
  draft.value = "keep"; native.recoverConnection.mockImplementationOnce(() => { throw Error("bridge unavailable"); });
  model.recover(); expect(model.busy.value).toBe(false); expect(model.error.value).toBe("bridge unavailable");
  expect(draft.value).toBe("keep"); model.recover(); expect(model.busy.value).toBe(true);
});
it("disposal removes its listener, cancels the active request and leaves the draft untouched", async () => {
  const { native, draft, model, request, event } = setup("keep"); model.recover(); const id = request();
  scope.stop(); event(id, "error", "late"); await vi.advanceTimersByTimeAsync(20000);
  expect(native.cancelConnectionRecovery).toHaveBeenCalledExactlyOnceWith(id); expect(model.error.value).toBe(""); expect(draft.value).toBe("keep");
});
