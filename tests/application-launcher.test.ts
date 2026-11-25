import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { computed, effectScope, nextTick, ref, type EffectScope } from "vue";
import { useApplications, type LauncherEntry } from "../apps/mobile/composables/useApplications";
import { languagePreference, t } from "../shared/i18n";
import type { Command } from "../shared/protocol";

let scope: EffectScope;
const entries: LauncherEntry[] = [
  { id: "a".repeat(32), name: "First app", kind: "application", available: true },
  { id: "b".repeat(32), name: "Second folder", kind: "folder", available: true },
];
const catalog = (items = entries) => Response.json({ applications: items });
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
beforeEach(() => { scope = effectScope(); languagePreference.value = "zh-CN"; });
afterEach(() => { scope.stop(); languagePreference.value = "system"; });
async function setup(queued = false) {
  const panel = ref<string | null>("applications"), allowed = ref(true);
  const read = vi.fn<(init?: RequestInit) => Promise<Response>>().mockImplementation(async () => Response.json({ applications: entries, launchJobs: queued }));
  const status = vi.fn<(init?: RequestInit) => Promise<Response>>().mockResolvedValue(Response.json({ state: "succeeded", message: "" }));
  const api = vi.fn((path: string, init?: RequestInit) => path.endsWith("/icons") ? Promise.resolve(Response.json({ icons: [] })) : path.includes("/launches/") ? status(init) : read(init));
  const send = vi.fn<(command: Command) => Promise<void>>().mockResolvedValue(undefined);
  const model = scope.run(() => useApplications({ api, send }, panel, computed(() => allowed.value)))!;
  await model.refreshApplications();
  return { panel, allowed, read, status, send, api, model };
}
it("ends Sending when the launch fails while the optional status check is still waiting", async () => {
  const { model, read, send } = await setup(), lookup = deferred<Response>();
  read.mockReturnValueOnce(lookup.promise); send.mockRejectedValueOnce(new Error("操作结果未知，请确认电脑状态；不会自动重发"));
  const operation = model.launchApplication(model.applications.value[0]);
  await vi.waitFor(() => expect(model.applicationActionError.value).toContain("不会自动重发"));
  expect(model.applicationPending.value).toBe("");
  expect(model.applicationsReady.value).toBe(true);
  lookup.resolve(catalog([{ ...entries[0], available: false }, entries[1]])); await operation;
  expect(model.applicationActionError.value).toContain("不会自动重发");
  expect(model.applicationActionError.value).toContain("应用文件已移动或删除");
  expect(model.applications.value[0].available).toBe(false);
  expect(send).toHaveBeenCalledTimes(1);
});
it("an old status response and its cleanup cannot overwrite or unlock a newer launch", async () => {
  const { model, read, send } = await setup(), lookup = deferred<Response>(), second = deferred<void>();
  read.mockReturnValueOnce(lookup.promise); send.mockRejectedValueOnce(new Error("first failed")).mockReturnValueOnce(second.promise);
  const first = model.launchApplication(model.applications.value[0]);
  await vi.waitFor(() => expect(model.applicationPending.value).toBe(""));
  const signal = read.mock.calls[1][0]?.signal;
  const next = model.launchApplication(model.applications.value[1]);
  expect(signal?.aborted).toBe(true);
  lookup.resolve(catalog([{ ...entries[0], available: false }])); await first;
  expect(model.applicationPending.value).toBe(entries[1].id);
  expect(model.applicationActionError.value).toBe("");
  expect(model.applications.value[0].available).toBe(true);
  second.resolve(); await next;
  expect(model.applicationsNotice.value).toContain(entries[1].name);
  expect(model.applicationPending.value).toBe("");
});
it("refresh cancels an old failure lookup and keeps the refreshed catalog", async () => {
  const { model, read, send } = await setup(), lookup = deferred<Response>();
  read.mockReturnValueOnce(lookup.promise); send.mockRejectedValueOnce(new Error("failed"));
  const first = model.launchApplication(model.applications.value[0]);
  await vi.waitFor(() => expect(model.applicationActionError.value).toBe("failed"));
  await model.refreshApplications();
  lookup.resolve(catalog([])); await first;
  expect(model.applications.value).toHaveLength(2);
  expect(model.applicationActionError.value).toBe("");
});
it("removed entries retain the command result and report availability in every language", async () => {
  const { model, read, send } = await setup();
  read.mockResolvedValueOnce(catalog([])); send.mockRejectedValueOnce(new Error("操作结果未知，请确认电脑状态；不会自动重发"));
  await model.launchApplication(model.applications.value[0]);
  expect(model.applications.value.map(entry => entry.id)).toEqual([entries[1].id]);
  for (const language of ["zh-CN", "zh-TW", "en"]) {
    languagePreference.value = language;
    expect(model.applicationActionError.value).toBe(t("操作结果未知，请确认电脑状态；不会自动重发") + "\n" + t("快捷入口已移除，请刷新列表"));
  }
});
it("duplicate taps and refresh cannot resend a command that is still in flight", async () => {
  const { model, read, send } = await setup(), sending = deferred<void>(); send.mockReturnValueOnce(sending.promise);
  const first = model.launchApplication(model.applications.value[0]);
  await model.launchApplication(model.applications.value[0]); await model.refreshApplications();
  expect(send).toHaveBeenCalledTimes(1); expect(read).toHaveBeenCalledTimes(1);
  sending.resolve(); await first;
});
it("leaving the panel prevents a late lookup from changing the list", async () => {
  const { model, read, send, panel } = await setup(), lookup = deferred<Response>();
  read.mockReturnValueOnce(lookup.promise); send.mockRejectedValueOnce(new Error("failed"));
  const first = model.launchApplication(model.applications.value[0]);
  await vi.waitFor(() => expect(model.applicationActionError.value).toBe("failed"));
  panel.value = null; await nextTick(); lookup.resolve(catalog([])); await first;
  expect(model.applications.value).toHaveLength(2); expect(model.applicationActionError.value).toBe("");
});
it("a failure lookup cannot change a new catalog after permission is revoked and restored", async () => {
  const { model, read, send, allowed } = await setup(), lookup = deferred<Response>();
  read.mockReturnValueOnce(lookup.promise); send.mockRejectedValueOnce(new Error("failed"));
  const first = model.launchApplication(model.applications.value[0]);
  await vi.waitFor(() => expect(model.applicationActionError.value).toBe("failed"));
  allowed.value = false; await nextTick(); expect(model.applications.value).toHaveLength(0);
  allowed.value = true; await vi.waitFor(() => expect(model.applications.value).toHaveLength(2));
  lookup.resolve(catalog([])); await first;
  expect(model.applications.value).toHaveLength(2); expect(model.applicationActionError.value).toBe("failed");
});
it("queue admission is followed by Opening and the matching receipt, without resending", async () => {
  const { model, status, send, api } = await setup(true);
  status.mockResolvedValueOnce(Response.json({ state: "preparing", message: "" }))
    .mockResolvedValueOnce(Response.json({ state: "starting", message: "" }));
  const operation = model.launchApplication(model.applications.value[0]);
  await vi.waitFor(() => expect(model.applicationPendingDetail.value).toBe("正在打开…"));
  expect(model.applicationsNotice.value).toBe("");
  await operation;
  expect(model.applicationsNotice.value).toContain(entries[0].name);
  expect(send).toHaveBeenCalledTimes(1);
  const command = send.mock.calls[0][0];
  expect(command.type).toBe("launch");
  if (command.type !== "launch") throw new Error("wrong command");
  expect(command.request).toMatch(/^[a-f0-9]{32}$/);
  expect(api.mock.calls.filter(([path]) => path.includes("/launches/")).map(([path]) => path)).toEqual(Array(3).fill("/api/applications/launches/" + command.request));
});
it("a known launch failure does not claim success or become an uncertain QFT outcome", async () => {
  const { model, status, send } = await setup(true);
  status.mockResolvedValueOnce(Response.json({ state: "failed", message: "应用文件已移动或删除，请在电脑端重新添加" }));
  await model.launchApplication(model.applications.value[0]);
  expect(model.applicationActionError.value).toBe("应用文件已移动或删除，请在电脑端重新添加");
  expect(model.applicationsNotice.value).toBe(""); expect(model.applicationPending.value).toBe(""); expect(send).toHaveBeenCalledTimes(1);
});
it.each([
  Response.json({ error: "not found" }, { status: 404 }),
  Response.json({ error: "denied" }, { status: 403 }),
  Response.json({ state: "unexpected", message: "" }),
])("missing, forbidden or malformed receipts stay uncertain without retrying the command", async response => {
  const { model, status, send } = await setup(true); status.mockResolvedValueOnce(response);
  await model.launchApplication(model.applications.value[0]);
  expect(model.applicationActionError.value).toBe("启动结果未确认，请检查电脑；不会自动重发");
  expect(model.applicationsNotice.value).toBe(""); expect(send).toHaveBeenCalledTimes(1);
});
function waitForAbort(init?: RequestInit) {
  return new Promise<Response>((_, reject) => init?.signal?.addEventListener("abort", () => reject(new Error("aborted")), { once: true }));
}
it("leaving a queued launch view cancels receipt polling and ignores its late result", async () => {
  const { model, status, send, panel } = await setup(true); status.mockImplementationOnce(waitForAbort);
  const operation = model.launchApplication(model.applications.value[0]);
  await vi.waitFor(() => expect(status).toHaveBeenCalledTimes(1));
  panel.value = null; await nextTick(); await operation;
  expect(model.applicationActionError.value).toBe(""); expect(model.applicationsNotice.value).toBe(""); expect(model.applicationPending.value).toBe(""); expect(send).toHaveBeenCalledTimes(1);
});
it("disconnecting while opening preserves uncertainty and never restarts the accepted job", async () => {
  const { model, status, send, allowed } = await setup(true); status.mockImplementationOnce(waitForAbort);
  const operation = model.launchApplication(model.applications.value[0]); await vi.waitFor(() => expect(status).toHaveBeenCalledTimes(1));
  allowed.value = false; await nextTick(); await operation;
  allowed.value = true; await nextTick();
  expect(model.applicationActionError.value).toBe("启动结果未确认，请检查电脑；不会自动重发"); expect(send).toHaveBeenCalledTimes(1);
});
it("receipt polling has a bounded deadline even when the server does not answer", async () => {
  vi.useFakeTimers();
  try {
    const { model, status, send } = await setup(true); status.mockImplementationOnce(waitForAbort);
    const operation = model.launchApplication(model.applications.value[0]); await vi.advanceTimersByTimeAsync(6001); await operation;
    expect(model.applicationActionError.value).toBe("启动结果未确认，请检查电脑；不会自动重发"); expect(send).toHaveBeenCalledTimes(1); expect(vi.getTimerCount()).toBe(0);
  } finally { vi.useRealTimers(); }
});
